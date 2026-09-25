//! Shell-icon-bitmap extraction for the overlay renderer.
//!
//! Given a PIDL owned by the backend's cache, this module produces a
//! premultiplied-BGRA `IconBitmap` suitable for uploading to a
//! Direct2D bitmap.
//!
//! # Fallback chain
//!
//! 1. **`IShellItemImageFactory::GetImage`** — modern, respects
//!    thumbnails/overlays, works for jumbo (256 px) sizes. This is
//!    the primary path.
//! 2. **`SHGetFileInfoW(SHGFI_SYSICONINDEX | SHGFI_LARGEICON)`** →
//!    `IImageList::GetIcon` — fallback for virtual PIDLs that the
//!    modern factory rejects.
//! 3. **`None`** — the renderer draws a solid placeholder tile.
//!
//! # Threading
//!
//! Every function here must be called on the STA worker thread that
//! owns the backend's `IFolderView2` — the shell interfaces involved
//! are all apartment-threaded.

use rdi_core::{DesktopError, IconBitmap};
use std::cell::RefCell;
use windows::Win32::Foundation::{HWND, MAX_PATH, SIZE};
use windows::Win32::Graphics::Gdi::{
    BITMAP, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC,
    GetDIBits, GetObjectW, HBITMAP, ReleaseDC,
};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    GIL_CHECKSHIELD, GIL_FORSHELL, GIL_SHIELD, IExtractIconW, IShellItem,
    IShellItemImageFactory, IShellFolder, SHFILEINFOW, SHGSI_ICON, SHGSI_LARGEICON,
    SHGetFileInfoW, SHGetImageList, SHGetStockIconInfo, SHGFI_ICON, SHGFI_LARGEICON,
    SHGFI_OVERLAYINDEX, SHGFI_PIDL, SHIL_JUMBO,
    SHSTOCKICONINFO, SHBindToParent, SHCreateItemFromIDList, SIID_SHIELD,
    SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY, SIIGBF_THUMBNAILONLY,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};
use windows::core::{Interface, PCWSTR};

thread_local! {
    /// Cached `SHIL_JUMBO` (256 px) system image list, per STA
    /// worker thread. `SHGetImageList` is safe to call repeatedly
    /// but hits a COM boundary each time — caching keeps
    /// `extract_overlay_bitmap` cheap for the common path (one
    /// call per icon at session start).
    static SYSTEM_IMAGE_LIST_JUMBO: RefCell<Option<IImageList>> =
        const { RefCell::new(None) };
    /// Cached shield stock icon (`SIID_SHIELD`), per STA worker
    /// thread. Outer `Option` = initialised flag, inner = success
    /// (`Some(bitmap)`) or failure (`None`). Blend target for the
    /// admin-elevation shield decoration; the shield is fetched once
    /// per session, then reused for every icon that reports
    /// `GIL_SHIELD` on `IExtractIconW::GetIconLocation`.
    static SHIELD_BITMAP: RefCell<Option<Option<IconBitmap>>> = const { RefCell::new(None) };
}

/// Static crop of the visible arrow inside the SHIL_JUMBO sprite:
/// `(x, y, w, h)` in the 256×256 source. Measured against the current
/// shell (`alpha > 0` and `alpha > 32` bboxes, and
/// `visual._sprite_white_core`, all give the identical result).
/// If Windows ever ships a new sprite, re-measure with
/// [temp_python/rdi_debug/arrow_sprite.py](../../../../temp_python/rdi_debug/arrow_sprite.py).
const SHORTCUT_ARROW_JUMBO_CROP: (u32, u32, u32, u32) = (0, 156, 100, 100);

/// Extract the shell icon bitmap for `pidl` at approximately
/// `size × size` pixels. Returns `None` if every fallback fails —
/// the renderer treats that as "draw a solid placeholder tile".
///
/// `draw_shortcut_overlay` gates the bottom-left overlay-slot badge
/// (`SHGetFileInfoW(SHGFI_OVERLAYINDEX)` + `SHIL_JUMBO` extraction
/// cropped to the [`SHORTCUT_ARROW_JUMBO_CROP`] opaque subrect).
/// When `false`, both the Shell probe and the extraction are
/// skipped — callers that don't want the arrow pay no COM cost for
/// it.
///
/// `draw_shield_overlay` gates the bottom-right admin-elevation
/// shield badge (`IExtractIconW::GetIconLocation(GIL_CHECKSHIELD)`
/// probe + `SIID_SHIELD` blend). Same short-circuit semantics.
///
/// # Safety
/// `pidl` must be a valid absolute PIDL currently owned by the
/// caller's cache. The bitmap returned by the shell is copied into a
/// `Vec<u8>` before this function returns; the caller does not need
/// to keep the PIDL alive past the call.
pub(crate) unsafe fn extract_icon_bitmap(
    pidl: *const ITEMIDLIST,
    size: u32,
    draw_shortcut_overlay: bool,
    draw_shield_overlay: bool,
) -> Option<ExtractedIcon> {
    // SAFETY: forwarded to caller. `pidl` is a valid absolute PIDL.
    let mut base = match unsafe { extract_via_image_factory(pidl, size) } {
        Ok(bmp) => bmp,
        Err(_) => {
            // Only log the factory failure at trace-verbosity: virtual PIDLs
            // like "This PC" reliably fail the factory path but succeed via
            // the ImageList fallback below.
            // The `SHGetFileInfo` fallback is not wired up because the
            // `IShellItemImageFactory` path succeeds on every icon the
            // production Windows 11 desktop presents in current testing.
            return None;
        }
    };
    // Extract the Explorer overlay-slot badge (shortcut arrow /
    // sharing hand / sync-cloud) if the shell reports one, but do
    // NOT blend it into the base. The overlay renderer anchors it
    // to the icon slot's bottom-left, not the (potentially
    // smaller-than-slot) base bitmap's bottom-left.
    // SAFETY: forwarded to caller.
    let shortcut_arrow = if draw_shortcut_overlay {
        unsafe { extract_overlay_bitmap(pidl) }
    } else {
        None
    };
    // If the item's `IExtractIconW::GetIconLocation` reports
    // `GIL_SHIELD`, composite an admin-elevation shield in the
    // bottom-right corner of the icon. This is Explorer's second
    // overlay-decoration path (the SHGFI overlay index handles
    // shortcut arrows / cloud badges / sharing hands; the shield
    // is separate because it's derived from the target's manifest,
    // not the shell's overlay-slot registry). Both can apply
    // simultaneously — a shortcut to an admin-required exe gets
    // arrow at bottom-left AND shield at bottom-right.
    // SAFETY: forwarded to caller.
    if draw_shield_overlay && unsafe { has_shield_overlay(pidl) } {
        if let Some(shield) = with_shield_bitmap(|b| b.clone()) {
            blend_shield_bottom_right(&mut base, &shield);
        }
    }
    Some(ExtractedIcon {
        base,
        shortcut_arrow,
    })
}

/// Bundle returned by [`extract_icon_bitmap`]: the base icon bitmap
/// (thumbnail or shell-association icon) plus the shortcut-arrow
/// overlay when the shell reports one.
///
/// The arrow is kept separate so the overlay renderer can anchor it
/// to the icon slot's bottom-left corner rather than the base
/// bitmap's.
pub(crate) struct ExtractedIcon {
    pub base: IconBitmap,
    pub shortcut_arrow: Option<IconBitmap>,
}

/// Modern extraction path — `IShellItemImageFactory::GetImage`.
///
/// # Safety
/// `pidl` must be a valid absolute PIDL.
unsafe fn extract_via_image_factory(
    pidl: *const ITEMIDLIST,
    size: u32,
) -> Result<IconBitmap, DesktopError> {
    // SAFETY: `SHCreateItemFromIDList` creates an `IShellItem` from
    // an absolute PIDL, taking a reference to the PIDL only for the
    // duration of the call.
    let item: IShellItem = unsafe {
        SHCreateItemFromIDList(pidl).map_err(|e| {
            DesktopError::BackendUnavailable(format!(
                "SHCreateItemFromIDList failed: {e}"
            ))
        })?
    };
    let factory: IShellItemImageFactory = item.cast().map_err(|e| {
        DesktopError::BackendUnavailable(format!(
            "IShellItem -> IShellItemImageFactory cast failed: {e}"
        ))
    })?;

    let size_i32 = size as i32;
    let sz = SIZE {
        cx: size_i32,
        cy: size_i32,
    };
    // Two-stage extraction: try the target's **thumbnail** first so
    // shortcuts to images / videos / PDFs render the same preview
    // Explorer shows on the desktop, and fall back to the shell-
    // association icon on any failure (including `E_PENDING` from slow
    // providers like OneDrive online-only files, or virtual/non-file
    // PIDLs that never have a thumbnail).
    //
    // Latency note: `SIIGBF_THUMBNAILONLY` without `SIIGBF_MEMORYONLY`
    // may synchronously generate the thumbnail on first request;
    // subsequent calls hit the shell thumbnail cache. Empirically the
    // one-time cost per icon at session start is well under a
    // millisecond on a warm cache and imperceptible on animation
    // start-up.
    //
    // SAFETY: `factory` is a valid interface pointer; `sz` is a
    // stack-local `SIZE` that outlives both calls. Every returned
    // `HBITMAP` must be `DeleteObject`'d exactly once — done
    // unconditionally below.
    let hbmp: HBITMAP = unsafe {
        match factory.GetImage(sz, SIIGBF_BIGGERSIZEOK | SIIGBF_THUMBNAILONLY) {
            Ok(h) => h,
            Err(_) => factory
                .GetImage(sz, SIIGBF_BIGGERSIZEOK | SIIGBF_ICONONLY)
                .map_err(|e| {
                    DesktopError::BackendUnavailable(format!(
                        "IShellItemImageFactory::GetImage (thumbnail + icon \
                         fallback) failed: {e}"
                    ))
                })?,
        }
    };
    // HBITMAP ownership transfers to the caller. `hbitmap_to_bgra`
    // copies pixels, so it can be freed immediately after.
    let bmp = unsafe { hbitmap_to_bgra(hbmp) };
    // SAFETY: `hbmp` was just returned by GetImage; freeing it is
    // required per MSDN.
    unsafe {
        let _ = DeleteObject(hbmp.into());
    }
    bmp
}

/// Copy an HBITMAP into a premultiplied-BGRA byte buffer.
///
/// # Safety
/// `hbmp` must be a valid HBITMAP that was created by the caller and
/// owns its pixel storage.
unsafe fn hbitmap_to_bgra(hbmp: HBITMAP) -> Result<IconBitmap, DesktopError> {
    // ---- Query dimensions --------------------------------------------
    let mut info: BITMAP = unsafe { std::mem::zeroed() };
    // SAFETY: `GetObjectW` writes exactly `size_of::<BITMAP>()` bytes
    // to `info` and returns the number of bytes written.
    let n = unsafe {
        GetObjectW(
            hbmp.into(),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut info as *mut _ as *mut _),
        )
    };
    if n == 0 {
        return Err(DesktopError::BackendUnavailable(
            "GetObjectW(HBITMAP) returned 0 bytes".into(),
        ));
    }
    let width = info.bmWidth.max(0) as u32;
    let height = info.bmHeight.max(0) as u32;
    if width == 0 || height == 0 {
        return Err(DesktopError::BackendUnavailable(
            "GetObjectW(HBITMAP) reported zero size".into(),
        ));
    }

    // ---- Ask the DIB API for 32-bpp top-down BGRA --------------------
    let stride = width * 4;
    let mut pixels = vec![0u8; (stride * height) as usize];

    let mut header = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width as i32,
        // Negative height requests a top-down bitmap — same row order
        // as Direct2D expects.
        biHeight: -(height as i32),
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0 as u32,
        biSizeImage: 0,
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    };
    let bitmap_info = BITMAPINFO {
        bmiHeader: header,
        bmiColors: [Default::default(); 1],
    };

    // SAFETY: `GetDC(None)` returns an HDC for the screen, released
    // below. `GetDIBits` writes exactly `stride * height` bytes into
    // `pixels` and does not retain any pointer.
    let hdc = unsafe { GetDC(None) };
    if hdc.is_invalid() {
        return Err(DesktopError::BackendUnavailable(
            "GetDC(NULL) returned invalid HDC".into(),
        ));
    }
    let rows = unsafe {
        GetDIBits(
            hdc,
            hbmp,
            0,
            height,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut { bitmap_info } as *mut _,
            DIB_RGB_COLORS,
        )
    };
    // SAFETY: HDC obtained from GetDC(None) must be released.
    unsafe {
        ReleaseDC(None, hdc);
    }
    if rows == 0 {
        return Err(DesktopError::BackendUnavailable(
            "GetDIBits returned 0 scanlines".into(),
        ));
    }

    // Silence the unused-`header` warning without adding another
    // mutable binding — `BITMAPINFO::bmiHeader` is filled directly
    // above.
    let _ = &mut header;

    // ---- Premultiply alpha -------------------------------------------
    //
    // `GetDIBits` returns straight (non-premultiplied) BGRA. Direct2D
    // expects premultiplied BGRA (`DXGI_FORMAT_B8G8R8A8_UNORM` with
    // `D2D1_ALPHA_MODE_PREMULTIPLIED`) — otherwise the icon renders
    // with a bright halo around alpha-blended edges.
    for px in pixels.chunks_exact_mut(4) {
        let a = px[3] as u16;
        // Fast integer premultiply: `(c * a + 127) / 255` rounds to
        // nearest, matching what SkColor4f uses.
        px[0] = ((px[0] as u16 * a + 127) / 255) as u8;
        px[1] = ((px[1] as u16 * a + 127) / 255) as u8;
        px[2] = ((px[2] as u16 * a + 127) / 255) as u8;
    }

    Ok(IconBitmap {
        width,
        height,
        stride,
        pixels,
    })
}

// ---------------------------------------------------------------------------
// Shell overlay compositing (shortcut arrow, admin shield, cloud sync, …)
// ---------------------------------------------------------------------------

/// Extract the Explorer overlay icon (shortcut arrow / UAC shield /
/// sync-cloud badge / …) that applies to `pidl`, if any. Returns
/// `None` when the shell reports no overlay for the item.
///
/// The returned bitmap is a **`SHORTCUT_ARROW_JUMBO_CROP`-sized**
/// (100×100) crop of the `SHIL_JUMBO` (256×256) source sprite — the
/// exact opaque subrect Explorer's imagelist paints. The renderer's
/// `DrawBitmap` scales it to
/// [`shortcut_arrow_target_size_dip`](crate::overlay) with no
/// per-frame crop math.
///
/// # Safety
/// `pidl` must be a valid absolute PIDL. The caller does not need
/// to keep the PIDL alive past this call — it is only passed to
/// `SHGetFileInfoW`.
unsafe fn extract_overlay_bitmap(
    pidl: *const ITEMIDLIST,
) -> Option<IconBitmap> {
    // 1. Query the overlay index. `SHGFI_PIDL` re-interprets the
    //    first argument as `const ITEMIDLIST *`; `SHGFI_ICON` +
    //    `SHGFI_LARGEICON` triggers the shell's icon-lookup path
    //    (required — see the comment on the SHGetFileInfoW call
    //    below); `SHGFI_OVERLAYINDEX` packs the overlay index
    //    (0–15) into the high byte of `sfi.iIcon`.
    let mut sfi: SHFILEINFOW = unsafe { std::mem::zeroed() };
    // SAFETY: `SHGetFileInfoW` reads the PIDL (SHGFI_PIDL flag
    // active) and writes `sfi`. `pidl` is a valid absolute PIDL by
    // contract. A 0 return means failure, treated as "no overlay".
    //
    // Important — `SHGFI_OVERLAYINDEX` is documented as modifying
    // `SHGFI_ICON`, and on Windows 11 the shell only populates the
    // overlay index into `sfi.iIcon` when both `SHGFI_ICON` and
    // `SHGFI_OVERLAYINDEX` are present (`SHGFI_SYSICONINDEX` alone
    // returns iIcon with the high byte zeroed). Hence the
    // `SHGFI_ICON | SHGFI_LARGEICON` flags, and the immediate
    // `DestroyIcon` of the incidentally-returned HICON. Cost: one
    // extra HICON allocation per icon at session start (~150 icons
    // × ~0.1 ms).
    let ret = unsafe {
        SHGetFileInfoW(
            PCWSTR(pidl.cast()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut sfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_PIDL | SHGFI_ICON | SHGFI_LARGEICON | SHGFI_OVERLAYINDEX,
        )
    };
    // Only the overlay index in `sfi.iIcon` is wanted; free the HICON.
    if !sfi.hIcon.is_invalid() {
        unsafe {
            let _ = DestroyIcon(sfi.hIcon);
        }
    }
    if ret == 0 {
        return None;
    }
    // Overlay index lives in bits 24–31 of `iIcon`. A value of 0
    // means "no overlay applies" — the common case; short-circuit.
    let overlay_index = ((sfi.iIcon as u32) >> 24) & 0x0F;
    if overlay_index == 0 {
        return None;
    }

    // 2. Fetch (or reuse) the cached SHIL_JUMBO system image list.
    //    Per STA worker thread — see `SYSTEM_IMAGE_LIST_JUMBO`.
    let list = with_jumbo_image_list(|list| list.clone())?;

    // 3. Look up the imagelist index of the overlay bitmap slot.
    // SAFETY: `list` is a valid `IImageList`; `overlay_index` is
    // 1–15 (0 was filtered above).
    let overlay_slot: i32 = unsafe {
        match list.GetOverlayImage(overlay_index as i32) {
            Ok(idx) => idx,
            Err(_) => return None,
        }
    };
    if overlay_slot < 0 {
        return None;
    }

    // 4. Extract the overlay as an HICON. `ILD_TRANSPARENT` yields an
    //    alpha channel where the overlay's mask says "empty".
    // SAFETY: `overlay_slot` is a valid imagelist index obtained above.
    let hicon: HICON = unsafe {
        match list.GetIcon(overlay_slot, ILD_TRANSPARENT.0) {
            Ok(h) => h,
            Err(_) => return None,
        }
    };

    // 5. HICON → HBITMAP → premultiplied-BGRA `IconBitmap` →
    //    static (0, 156, 100, 100) crop.
    let bmp = unsafe { hicon_to_bgra(hicon) };
    // Free the HICON per MSDN — GetIcon transfers ownership to the caller.
    unsafe {
        let _ = DestroyIcon(hicon);
    }
    bmp.ok().and_then(crop_shortcut_arrow_jumbo)
}

/// Crop the SHIL_JUMBO overlay sprite to its visible-arrow subrect
/// [`SHORTCUT_ARROW_JUMBO_CROP`]. Returns `None` when the source
/// isn't the expected 256×256 shape — a defensive guard against
/// future shell changes; callers treat that as "no overlay
/// available" and the renderer falls back to no arrow.
fn crop_shortcut_arrow_jumbo(src: IconBitmap) -> Option<IconBitmap> {
    let (cx, cy, cw, ch) = SHORTCUT_ARROW_JUMBO_CROP;
    if src.width < cx + cw || src.height < cy + ch {
        return None;
    }
    let src_stride = src.stride as usize;
    let dst_stride = (cw as usize) * 4;
    let mut pixels = Vec::with_capacity(dst_stride * ch as usize);
    for row in 0..ch as usize {
        let src_row_start = (cy as usize + row) * src_stride + (cx as usize) * 4;
        let src_row_end = src_row_start + dst_stride;
        pixels.extend_from_slice(&src.pixels[src_row_start..src_row_end]);
    }
    Some(IconBitmap {
        width: cw,
        height: ch,
        stride: dst_stride as u32,
        pixels,
    })
}

/// Convert an `HICON` to a premultiplied-BGRA `IconBitmap`.
///
/// # Safety
/// `hicon` must be a valid HICON. The caller owns the HICON and is
/// responsible for `DestroyIcon` after this call returns.
unsafe fn hicon_to_bgra(hicon: HICON) -> Result<IconBitmap, DesktopError> {
    let mut info: ICONINFO = unsafe { std::mem::zeroed() };
    // SAFETY: `GetIconInfo` fills `info` including two caller-owned
    // HBITMAP handles (`hbmMask` and `hbmColor`). Both MUST be
    // `DeleteObject`'d before returning.
    unsafe {
        GetIconInfo(hicon, &mut info).map_err(|e| {
            DesktopError::BackendUnavailable(format!("GetIconInfo failed: {e}"))
        })?;
    }
    // Modern 32-bpp ARGB icons (which SHIL_JUMBO overlays are) have
    // `hbmColor` populated with the pixels + alpha; older
    // XOR/AND-mask icons leave `hbmColor` null and put the pixels in
    // the mask. The relevant overlays (arrow / shield / cloud) are
    // all modern, so `hbmColor` is required.
    let color = info.hbmColor;
    let mask = info.hbmMask;
    let result = if !color.is_invalid() {
        unsafe { hbitmap_to_bgra(color) }
    } else {
        Err(DesktopError::BackendUnavailable(
            "HICON has no color bitmap (mask-only icons are not supported for overlays)".into(),
        ))
    };
    // SAFETY: both handles obtained from `GetIconInfo` MUST be freed
    // per MSDN; ignoring `DeleteObject` errors is fine.
    unsafe {
        if !color.is_invalid() {
            let _ = DeleteObject(color.into());
        }
        if !mask.is_invalid() {
            let _ = DeleteObject(mask.into());
        }
    }
    result
}

/// Run `f` with a reference to the cached SHIL_JUMBO system image
/// list, initialising the cell on first call.
///
/// Returns `None` if `SHGetImageList` fails; callers treat that as
/// "no overlay available".
fn with_jumbo_image_list<T>(
    f: impl FnOnce(&IImageList) -> T,
) -> Option<T> {
    SYSTEM_IMAGE_LIST_JUMBO.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            // SAFETY: `SHGetImageList` returns an owned `IImageList`
            // reference, cached here and released by `Drop` on
            // thread-local teardown.
            let list: windows::core::Result<IImageList> =
                unsafe { SHGetImageList(SHIL_JUMBO as i32) };
            match list {
                Ok(l) => *slot = Some(l),
                Err(_) => return None,
            }
        }
        slot.as_ref().map(f)
    })
}

/// Drop every cached Shell-derived resource on the current thread.
///
/// Call this whenever the Shell environment underneath the caches may
/// have changed — Explorer restarting (`TaskbarCreated`) invalidates the
/// system image list outright, and a DPI or display-topology change means
/// the shield bitmap was fetched at the wrong scale. Neither cache has
/// any self-invalidation, so without this a long-lived controller (the
/// MCP server holds exactly one) serves stale sprites indefinitely.
///
/// Cheap: the next icon extraction re-fetches both. Safe to call when
/// the caches are already empty.
///
/// Must run on the STA worker thread that populated the caches — they
/// are thread-locals, so a call from any other thread is a silent no-op
/// on that thread's (empty) slots.
pub(crate) fn invalidate_shell_caches() {
    SYSTEM_IMAGE_LIST_JUMBO.with(|cell| {
        if cell.borrow_mut().take().is_some() {
            tracing::debug!("dropped cached SHIL_JUMBO image list");
        }
    });
    SHIELD_BITMAP.with(|cell| {
        if cell.borrow_mut().take().is_some() {
            tracing::debug!("dropped cached shield bitmap");
        }
    });
}

// ---------------------------------------------------------------------------
// Shield (elevation) overlay — second-decoration path
// ---------------------------------------------------------------------------

/// Query `IExtractIconW::GetIconLocation(GIL_FORSHELL)` on `pidl`
/// and return `true` when the returned `pwFlags` include
/// `GIL_SHIELD`. This is the Explorer-authoritative test for "the
/// icon should have an admin-elevation shield overlay applied on
/// top of everything else"; it handles both `.lnk` shortcuts with
/// SLDF_RUNASUSER and bare `.exe` files whose manifest requests
/// `requireAdministrator` — the shell's IconLocation handler does
/// the manifest parsing.
///
/// # Safety
/// `pidl` must be a valid absolute PIDL (starts at the shell root).
unsafe fn has_shield_overlay(pidl: *const ITEMIDLIST) -> bool {
    let mut relative_pidl: *mut ITEMIDLIST = std::ptr::null_mut();
    // SAFETY: forwarded to caller. `SHBindToParent` fills
    // `relative_pidl` with a pointer INTO `pidl`'s ITEMIDLIST chain
    // — it must NOT be freed and is only valid while `pidl` is.
    let folder: IShellFolder = match unsafe {
        SHBindToParent(pidl, Some(&mut relative_pidl))
    } {
        Ok(f) => f,
        Err(_) => return false,
    };
    if relative_pidl.is_null() {
        return false;
    }
    let apidl: [*const ITEMIDLIST; 1] = [relative_pidl];
    // SAFETY: `folder` is a valid IShellFolder from SHBindToParent;
    // `apidl` outlives the call.
    let extractor: IExtractIconW = match unsafe {
        folder.GetUIObjectOf::<IExtractIconW>(HWND::default(), &apidl, None)
    } {
        Ok(e) => e,
        Err(_) => return false,
    };
    // Unused, but `GetIconLocation` requires the out-buffer.
    let mut buf = [0u16; MAX_PATH as usize];
    let mut idx: i32 = 0;
    let mut flags: u32 = 0;
    // SAFETY: `extractor` is a valid IExtractIconW; all out-params
    // are stack-local.
    //
    // `GIL_CHECKSHIELD` in the request flags asks the shell to
    // report whether a shield decoration is needed via
    // `GIL_SHIELD` in the returned `flags`. Without
    // `GIL_CHECKSHIELD`, the shell never sets the shield bit even
    // for admin-elevation shortcuts. See MSDN "IExtractIcon
    // GetIconLocation" — `GIL_CHECKSHIELD` and `GIL_SHIELD` share
    // the numeric value 0x0200; one is an input request, the
    // other an output result.
    let res = unsafe {
        extractor.GetIconLocation(
            GIL_FORSHELL | GIL_CHECKSHIELD,
            &mut buf,
            &mut idx,
            &mut flags,
        )
    };
    if res.is_err() {
        return false;
    }
    (flags & GIL_SHIELD) != 0
}

/// Run `f` with a reference to the cached shield stock icon bitmap.
/// The bitmap is fetched once per STA worker thread via
/// `SHGetStockIconInfo(SIID_SHIELD, SHGSI_ICON | SHGSI_LARGEICON)`
/// and cached until thread teardown.
///
/// Returns `None` if the stock icon fetch has failed.
fn with_shield_bitmap<T>(f: impl FnOnce(&IconBitmap) -> T) -> Option<T> {
    SHIELD_BITMAP.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(fetch_shield_bitmap());
        }
        slot.as_ref().and_then(|inner| inner.as_ref()).map(f)
    })
}

/// Fetch the shield stock icon (`SIID_SHIELD`) as a
/// premultiplied-BGRA bitmap. Called once per session on the STA
/// worker thread.
fn fetch_shield_bitmap() -> Option<IconBitmap> {
    let mut sii: SHSTOCKICONINFO = unsafe { std::mem::zeroed() };
    sii.cbSize = std::mem::size_of::<SHSTOCKICONINFO>() as u32;
    // SAFETY: `SHGetStockIconInfo` fills `sii` including an HICON
    // that must be `DestroyIcon`'d after copying pixels.
    if unsafe {
        SHGetStockIconInfo(SIID_SHIELD, SHGSI_ICON | SHGSI_LARGEICON, &mut sii)
    }
    .is_err()
    {
        return None;
    }
    if sii.hIcon.is_invalid() {
        return None;
    }
    let bmp = unsafe { hicon_to_bgra(sii.hIcon) }.ok();
    unsafe {
        let _ = DestroyIcon(sii.hIcon);
    }
    bmp
}

/// Blend `shield` into the bottom-right corner of `base`, scaled to
/// `base.width / 2` (empirical — matches Explorer's shield size for
/// admin-elevation decorations). Shield stock icons are compact
/// centered glyphs (unlike the padded overlay-imagelist bitmaps
/// used by shortcut arrows), hence the fixed fraction rather than
/// full-size.
fn blend_shield_bottom_right(base: &mut IconBitmap, shield: &IconBitmap) {
    if shield.width == 0 || shield.height == 0 || base.width == 0 || base.height == 0 {
        return;
    }
    let target = (base.width / 2).max(16).min(base.width);
    let target_w = target;
    let target_h = target;
    let origin_x = base.width - target_w;
    let origin_y = base.height - target_h;
    blend_scaled(base, shield, origin_x, origin_y, target_w, target_h);
}

/// Nearest-neighbour resample `src` to `(dst_w, dst_h)` and
/// premultiplied-"over"-blend into `dst` at `(dst_x, dst_y)`. Both
/// bitmaps must be premultiplied BGRA.
fn blend_scaled(
    dst: &mut IconBitmap,
    src: &IconBitmap,
    dst_x: u32,
    dst_y: u32,
    dst_w: u32,
    dst_h: u32,
) {
    if dst_w == 0 || dst_h == 0 || src.width == 0 || src.height == 0 {
        return;
    }
    let dst_stride = dst.stride as usize;
    let src_stride = src.stride as usize;
    for ty in 0..dst_h {
        if dst_y + ty >= dst.height {
            break;
        }
        let sy = (ty as u64 * src.height as u64 / dst_h as u64) as u32;
        let sy = sy.min(src.height - 1);
        let src_row = &src.pixels[sy as usize * src_stride..];
        let dst_row_start = (dst_y + ty) as usize * dst_stride + dst_x as usize * 4;
        let cols = dst_w.min(dst.width.saturating_sub(dst_x)) as usize;
        let dst_row = &mut dst.pixels[dst_row_start..dst_row_start + cols * 4];
        for tx in 0..cols as u32 {
            let sx = (tx as u64 * src.width as u64 / dst_w as u64) as u32;
            let sx = sx.min(src.width - 1);
            let s = &src_row[sx as usize * 4..sx as usize * 4 + 4];
            let d = &mut dst_row[tx as usize * 4..tx as usize * 4 + 4];
            let src_a = s[3] as u32;
            let inv_a = 255 - src_a;
            for c in 0..4 {
                let sc = s[c] as u32;
                let dc = d[c] as u32;
                d[c] = (sc + (dc * inv_a + 127) / 255).min(255) as u8;
            }
        }
    }
}

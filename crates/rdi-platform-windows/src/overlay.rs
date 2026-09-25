//! Transparent Direct2D + DirectComposition overlay used for
//! animating desktop-icon copies without asking `IFolderView2` to
//! repaint per frame.
//!
//! # Threading and lifetime
//!
//! Everything in this module must live on the engine's STA worker
//! thread. D3D11 / DXGI / DComposition are apartment-agnostic
//! themselves, but they share the thread with `IFolderView2`, which
//! is STA-only — moving any of them off that thread would violate the
//! shell contract. `Send` is deliberately *not* implemented on
//! `OverlaySession` so the type-system prevents accidents.
//!
//! # Rendering
//!
//! Renders real Shell icon bitmaps and DirectWrite
//! labels for every animated icon. Bitmaps come from
//! `IShellItemImageFactory` via [`crate::imaging`]; label text comes
//! from the icon's cached display name rendered in the user's
//! system icon-title font via [`crate::text`]. Selection tints and
//! focus outlines are deliberately not rendered — the overlay is a
//! purely spatial effect.
//!
//! Fallback: any icon whose bitmap could not be extracted degrades
//! to a solid rounded-rectangle placeholder so
//! the animation still runs.
//!
//! # No wallpaper capture, no reparenting
//!
//! The overlay is a **top-level, non-topmost, non-activating,
//! layered** window. It is *not* a child of `Progman` or `WorkerW`;
//! `WorkerW` is left completely untouched.

use std::collections::HashMap;

use rdi_core::{DesktopError, IconId, IconRenderPlan, Point, SnapshotIconGeometry};
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateDevice, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_CPU_READ,
    D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_MAP_OPTIONS_READ, D2D1_ROUNDED_RECT,
    ID2D1Bitmap, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionSurface, IDCompositionTarget,
    IDCompositionVisual,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_TEXT_METRICS, IDWriteTextLayout,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM,
};
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGISurface};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    FindWindowW, GW_HWNDPREV, GetClassNameW, GetSystemMetrics, GetWindow, HCURSOR, HICON,
    HWND_NOTOPMOST, IsWindowVisible, MSG, PM_REMOVE, PeekMessageW, PostQuitMessage,
    RegisterClassExW, RegisterWindowMessageW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SetWindowPos, ShowWindow, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, SW_SHOWNOACTIVATE, TranslateMessage, WM_DESTROY, WM_DISPLAYCHANGE,
    WM_DPICHANGED, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{Interface, PCWSTR, w};
use windows_numerics::Vector2;

use crate::text::{build_label_layout, build_text_resources, TextResources};

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::OnceLock;

/// Window-class name for the overlay. Registered on first use.
const OVERLAY_CLASS: PCWSTR = w!("RustyDesktopIconsOverlay");

/// Placeholder icon-copy render size when there is no `IconBitmap`
/// and no explicit `size_px`
const FALLBACK_ICON_SIZE_DIP: f32 = 48.0;

/// Corner radius of the fallback rounded rectangle (also used for the
/// selection tint under real icons).
const CORNER_RADIUS_DIP: f32 = 6.0;

/// Vertical gap in DIPs between the icon bitmap and its label.
const LABEL_GAP_DIP: f32 = 4.0;

/// Divisor + minimum for the empirical DPI-scaled thumbnail
/// bottom inset (`round(slot_h / N).max(min)`). 20 gives 2 px @
/// 100 %, 5 px @ 200 %, 6 px @ 250 %, 10 px @ 400 % — matches the
/// visually-verified inset on a 250 % monitor.
const THUMBNAIL_BOTTOM_INSET_DIVISOR: f32 = 20.0;
const THUMBNAIL_BOTTOM_INSET_MIN_DIP: f32 = 2.0;

/// Flat pixel reserve for the Explorer drop-shadow footprint on
/// image / video / PDF thumbnails. Applied only when `fit_h < h`;
/// subtracted once from `icon_left` and once from `icon_top` so
/// the thumbnail shifts up-left by exactly this many DIPs. Live
/// measurement puts the shadow offset at ~2 physical px on both
/// axes regardless of DPI — the shell draws the shadow via a
/// DirectComposition effect whose offset is DPI-independent, so a
/// DPI-scaled formula is wrong here.
const THUMBNAIL_SHADOW_RESERVE_DIP: f32 = 2.0;

// ---------------------------------------------------------------------------
// Cancellation plumbing
//
// The overlay window's WNDPROC runs on the same STA worker thread that
// owns the overlay session. When a shell-disruption message arrives
// (`TaskbarCreated`, `WM_DISPLAYCHANGE`, `WM_DPICHANGED`) the WNDPROC
// writes a non-zero value into the atom below. The next call into
// `WindowsBackend::commit_overlay_frame` pumps the message queue,
// drains the atom, and returns `DesktopError::OverlayCancelled` so the
// engine can end the animation gracefully.
//
// A static atom is deliberate: only one overlay session can exist per
// process at a time (enforced by `WindowsBackend::overlay: Option<...>`)
// so the atom's state cannot bleed between concurrent sessions.
// `OverlaySession::new` resets it to zero so a leftover value from a
// prior session cannot leak into a fresh one.
// ---------------------------------------------------------------------------

/// Reasons the overlay may be cancelled mid-animation. Serialised
/// into `OVERLAY_CANCEL_REASON` as a `u8`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum CancelReason {
    /// The `TaskbarCreated` broadcast fired — Explorer restarted and
    /// every cached `IFolderView2` / HWND / PIDL is stale.
    ExplorerRestarted = 1,
    /// `WM_DISPLAYCHANGE` — monitor added/removed or resolution
    /// changed. Overlay bounds and per-monitor DPI values are stale.
    DisplayChanged = 2,
    /// `WM_DPICHANGED` — the overlay's monitor changed scale. Per-
    /// icon sizes computed at enrichment time no longer match what
    /// the shell would render.
    DpiChanged = 3,
    /// Not a Windows message — synthesised by `commit_overlay_frame`
    /// when `IsWindow(cached_shell_view_hwnd)` returns FALSE. Covers
    /// the corner case where Explorer's shell view disappears
    /// without emitting `TaskbarCreated` (rare but observed on
    /// hostile shell replacements).
    ShellViewLost = 4,
}

impl CancelReason {
    /// Human-readable description propagated through
    /// `DesktopError::OverlayCancelled`.
    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::ExplorerRestarted => "Explorer restarted (TaskbarCreated broadcast)",
            Self::DisplayChanged => "display topology changed (WM_DISPLAYCHANGE)",
            Self::DpiChanged => "monitor DPI changed (WM_DPICHANGED)",
            Self::ShellViewLost => "shell view HWND is no longer valid",
        }
    }
}

/// Atomic slot the WNDPROC writes to. Zero means "no cancellation
/// pending"; any other value is a `CancelReason` discriminant.
static OVERLAY_CANCEL_REASON: AtomicU8 = AtomicU8::new(0);

/// The runtime-registered message ID for `"TaskbarCreated"`. Looked up
/// once via `RegisterWindowMessageW` and cached because the WNDPROC is
/// on the hot path.
static TASKBAR_CREATED_MSG: OnceLock<u32> = OnceLock::new();

/// `true` while some [`OverlaySession`] exists anywhere in this process.
///
/// Everything `static` in this module — the cancel atom, the window
/// class, the process DPI-awareness mode — is process-scoped, but
/// `WindowsBackend::overlay` is per-*instance*. Nothing stops a caller
/// from building two `DesktopController`s (the Python constructor is an
/// ordinary `__init__`), which yields two worker STAs sharing one
/// cancel atom: a disruption signalled against one session could be
/// consumed by the other. [`OverlayClaim`] makes that impossible.
static OVERLAY_CLAIMED: AtomicBool = AtomicBool::new(false);

/// RAII proof that this thread owns the process-wide overlay slot.
///
/// Held as a field of [`OverlaySession`], so the release rides the
/// existing drop chain and covers the panic path for free.
struct OverlayClaim;

impl OverlayClaim {
    /// Claim the process-wide overlay slot, or fail if another session
    /// already holds it.
    fn acquire() -> Result<Self, DesktopError> {
        OVERLAY_CLAIMED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| {
                DesktopError::OverlayUnavailable(
                    "another overlay session is already active in this process"
                        .into(),
                )
            })
    }
}

impl Drop for OverlayClaim {
    fn drop(&mut self) {
        OVERLAY_CLAIMED.store(false, Ordering::Release);
    }
}

/// Reset the cancel atom to "no cancellation". Called by
/// [`OverlaySession::new`] so a leftover value from a prior session
/// cannot bleed into a fresh one.
#[inline]
fn reset_cancel_state() {
    OVERLAY_CANCEL_REASON.store(0, Ordering::Release);
}

/// Take-and-clear the cancel reason. Returns `None` if no
/// cancellation is pending.
///
/// Uses `swap` so two WNDPROC invocations racing to set the value
/// while a commit is draining still yield exactly one consumption —
/// the second needs another commit turn to be seen. Realistic
/// scenarios always have < 1 event per commit tick.
#[inline]
pub(crate) fn take_cancel_reason() -> Option<CancelReason> {
    match OVERLAY_CANCEL_REASON.swap(0, Ordering::AcqRel) {
        1 => Some(CancelReason::ExplorerRestarted),
        2 => Some(CancelReason::DisplayChanged),
        3 => Some(CancelReason::DpiChanged),
        4 => Some(CancelReason::ShellViewLost),
        _ => None,
    }
}

/// Set the cancel reason from anywhere (WNDPROC or watchdog).
/// First writer wins — subsequent events during the same commit
/// window are dropped, which is fine because any reason triggers
/// the same "unwind and finalize" path.
///
/// Every cancel reason also invalidates the thread's cached
/// Shell resources: `ExplorerRestarted` / `ShellViewLost` make the
/// cached `IImageList` stale outright, and `DisplayChanged` /
/// `DpiChanged` mean the cached shield bitmap was fetched at the
/// wrong scale.
#[inline]
pub(crate) fn signal_cancel(reason: CancelReason) {
    let code = reason as u8;
    // `compare_exchange` only writes if the slot is still zero, so the
    // first reason to arrive wins.
    let first = OVERLAY_CANCEL_REASON
        .compare_exchange(0, code, Ordering::AcqRel, Ordering::Acquire)
        .is_ok();
    if first {
        crate::imaging::invalidate_shell_caches();
    }
}

/// Pump every queued message for the current thread through
/// `DispatchMessageW` so the WNDPROC has a chance to observe broadcast
/// messages like `WM_SETTINGCHANGE`. Returns after the queue is
/// drained.
///
/// # Safety
/// Must be called on the STA worker thread that owns the overlay
/// window. `PeekMessageW` with `hwnd=None` matches all windows owned
/// by the current thread, which here is exactly the overlay HWND.
pub(crate) fn pump_pending_messages() {
    // SAFETY: `msg` outlives every call; `PeekMessageW` writes into
    // it; `DispatchMessageW` reads it. Both are safe for any HWND.
    unsafe {
        let mut msg = MSG::default();
        // Cap at 64 messages per pump so a broken app spamming
        // broadcasts can't starve the tick loop.
        for _ in 0..64 {
            if !PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Owns an overlay HWND until the [`OverlaySession`] that will take
/// responsibility for it has actually been constructed.
///
/// `OverlaySession::new` creates the window and then runs ~14 fallible
/// steps (`D3D11CreateDevice` … `CreateSurface`) before it can build
/// the value whose `Drop` destroys that window. Without this guard
/// every one of those `?` leaks a top-level window — and because
/// `begin_overlay_session` failure is handled *gracefully* by the
/// engine's fallback path, the leak repeats once per `animate()` call.
struct HwndGuard(HWND);

impl HwndGuard {
    /// Hand the HWND over to a caller that will destroy it itself.
    fn disarm(mut self) -> HWND {
        std::mem::replace(&mut self.0, HWND::default())
    }
}

impl Drop for HwndGuard {
    fn drop(&mut self) {
        if self.0.0.is_null() {
            return;
        }
        // SAFETY: the HWND was created with `CreateWindowExW` on this
        // thread and no `OverlaySession` has taken ownership of it.
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

/// Live overlay state — one instance per active animation.
///
/// Every field is dropped in the reverse order it was constructed;
/// COM interfaces are released while the STA is still valid because
/// `OverlaySession` is `!Send` and is guaranteed to be dropped on the
/// worker thread that constructed it.
pub(crate) struct OverlaySession {
    hwnd: HWND,
    sprites: Option<crate::sprites::SpriteRenderer>,
    visual_frame: Vec<rdi_core::IconFrame>,
    background: Vec<(IconId, Point)>,
    _d3d_device: ID3D11Device,
    _dxgi_device: IDXGIDevice,
    _d2d_device: ID2D1Device,
    d2d_context: ID2D1DeviceContext,
    dcomp_device: IDCompositionDevice,
    _dcomp_target: IDCompositionTarget,
    _root_visual: IDCompositionVisual,

    /// The single DComp render surface, created once and reused for
    /// every frame.
    surface: IDCompositionSurface,

    // ---- Brushes -----------------------------------------------------
    /// Fallback placeholder tile (semi-transparent cyan).
    placeholder_brush: ID2D1SolidColorBrush,
    /// White brush for label glyphs.
    label_brush: ID2D1SolidColorBrush,
    /// Black brush for the label drop shadow.
    label_shadow_brush: ID2D1SolidColorBrush,

    // ---- Per-session enrichment cache --------------------------------
    /// Per-icon draw record (bitmap, label).
    icons: HashMap<IconId, IconEntry>,

    /// DirectWrite factory + IDWriteTextFormat.
    ///
    /// Held for the session lifetime so the `IDWriteTextLayout`
    /// objects cached inside `icons` (which reference the format)
    /// stay valid. Rust field-drop order releases `icons` — and
    /// therefore all layouts — before this factory.
    #[allow(dead_code)]
    text: TextResources,

    /// Overlay bounds in the virtual screen coordinate space.
    bounds: RECT,

    /// Whether the overlay window has been shown yet. The first
    /// `commit_frame` call makes it visible.
    shown: bool,

    /// Process-wide exclusivity. Declared **last** so it is dropped
    /// last (fields drop in declaration order) — the slot only frees
    /// up once every OS resource above has been released, so a
    /// waiting caller can never race a half-torn-down session.
    _claim: OverlayClaim,
}

/// Per-icon renderable state built once at session start.
pub(crate) struct IconEntry {
    /// Rendered icon bitmap (`None` for placeholder fallback).
    bitmap: Option<ID2D1Bitmap>,
    /// Icon render size in DIPs (independent of DPI — D2D handles
    /// physical scaling). This is the **slot** size — the space the
    /// icon occupies in the layout. The bitmap is aspect-fit
    /// centered inside this slot at [`Self::intrinsic_size_dip`];
    /// see [`draw_icon`].
    size_dip: (f32, f32),
    /// Bitmap's **intrinsic** pixel dimensions (populated from
    /// `IconBitmap.width / .height`). Equals [`Self::size_dip`] when
    /// there is no image or the shell returned a bitmap at the
    /// requested slot size. When Explorer's thumbnail provider
    /// returns a smaller bitmap than the requested slot — the
    /// common case for image / video / PDF shortcuts at ≥ 150 %
    /// DPI — this holds the returned bitmap's own dimensions, and
    /// [`draw_icon`] centers that bitmap inside the slot without
    /// upscaling.
    intrinsic_size_dip: (f32, f32),
    /// Optional label layout; `None` when the icon has no cached
    /// display name.
    label_layout: Option<IDWriteTextLayout>,
    /// Label bounds in DIPs — used to reserve space beneath the
    /// icon.
    label_size_dip: (f32, f32),
    /// Per-icon render offset in physical pixels (== DIPs at the
    /// overlay's 96-DPI-pinned coordinate space). Applied on top of
    /// the position passed to `draw_icon`. Empty `(0, 0)` for
    /// backends that don't populate it.
    render_offset_dip: (f32, f32),
    /// Optional shortcut-arrow overlay bitmap, uploaded separately
    /// so `draw_icon` can anchor it to the actual thumbnail's
    /// bottom-left (mirrored by the same `shadow_reserve_dip` the
    /// base bitmap uses, so the arrow lives in the shadow gutter
    /// below-left of the thumbnail). For slot-filling icons the
    /// anchor coincides with the slot bottom-left.
    ///
    /// The arrow's drawn size is derived from the slot's DIP
    /// width via [`shortcut_arrow_target_size_dip`], not the
    /// sprite's intrinsic dimensions.
    shortcut_arrow_bitmap: Option<ID2D1Bitmap>,
}

/// Errors surfaced by the overlay module map to `DesktopError`.
fn overlay_err(context: &str, e: windows::core::Error) -> DesktopError {
    DesktopError::OverlayUnavailable(format!(
        "{context}: HRESULT 0x{:08X} — {}",
        e.code().0 as u32,
        e.message()
    ))
}

/// Ensure the process is Per-Monitor V2 DPI aware.
pub(crate) fn ensure_dpi_awareness() {
    // SAFETY: safe from any thread. Failure only returns FALSE
    // (awareness already set to a different level).
    let _ = unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
    };
}

/// Register the overlay window class once (idempotent).
pub(crate) fn register_window_class() -> Result<(), DesktopError> {
    let hinstance = unsafe { GetModuleHandleW(None) }
        .map_err(|e| overlay_err("GetModuleHandleW", e))?;

    let wnd_class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(overlay_wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance.into(),
        hIcon: HICON::default(),
        hCursor: HCURSOR::default(),
        hbrBackground: HBRUSH::default(),
        lpszMenuName: PCWSTR::null(),
        lpszClassName: OVERLAY_CLASS,
        hIconSm: HICON::default(),
    };
    // SAFETY: `wnd_class` outlives the call. RegisterClassExW returns
    // 0 on failure; ERROR_CLASS_ALREADY_EXISTS (0x8007_0582) is fine.
    let atom = unsafe { RegisterClassExW(&wnd_class) };
    if atom == 0 {
        let err = windows::core::Error::from_thread();
        if err.code().0 as u32 != 0x8007_0582 {
            return Err(overlay_err("RegisterClassExW", err));
        }
    }
    Ok(())
}

/// Overlay window procedure.
///
/// The overlay is paint-only and input-transparent — every
/// non-shell-disruption message goes straight to `DefWindowProcW`.
///
/// Intercept the documented shell-disruption
/// broadcasts / notifications and stash a [`CancelReason`] in
/// [`OVERLAY_CANCEL_REASON`]. `WindowsBackend::commit_overlay_frame`
/// picks the value up on the next tick and returns
/// `DesktopError::OverlayCancelled` so the engine can end the
/// animation gracefully.
///
/// # Safety
/// Standard WNDPROC contract — the OS calls this on the thread that
/// created the window.
unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Cache the runtime-registered `TaskbarCreated` message ID on
    // first use. `RegisterWindowMessageW` returns the same ID for
    // the same name process-wide, so memoising is safe.
    //
    // SAFETY: `RegisterWindowMessageW` is safe with a NUL-terminated
    // wide string; the argument is a compile-time `w!` literal.
    let taskbar_msg = *TASKBAR_CREATED_MSG.get_or_init(|| unsafe {
        RegisterWindowMessageW(w!("TaskbarCreated"))
    });

    if taskbar_msg != 0 && msg == taskbar_msg {
        signal_cancel(CancelReason::ExplorerRestarted);
    } else if msg == WM_DISPLAYCHANGE {
        signal_cancel(CancelReason::DisplayChanged);
    } else if msg == WM_DPICHANGED {
        signal_cancel(CancelReason::DpiChanged);
    }

    if msg == WM_DESTROY {
        // SAFETY: `PostQuitMessage` takes an i32 exit code. No nested
        // message loop runs here, so the code itself is irrelevant.
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    // SAFETY: DefWindowProcW is the standard fallback; every argument
    // is valid by contract.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn virtual_screen_bounds() -> RECT {
    let x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let w = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let h = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    // NOTE: subtract 1 from `right` and `bottom` so the overlay is
    // 1 physical pixel smaller than the virtual screen in each
    // dimension.
    //
    // DWM's fullscreen-composition detector inspects layered
    // windows and flips the "app is running in full-screen mode"
    // signal whenever a WS_EX_LAYERED window's rect EXACTLY matches
    // a monitor / virtual-screen rect. When that flips, Windows
    // 10 / 11 Focus Assist auto-enables and suppresses notifications
    // for the duration of every animation.
    //
    // WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW and !WS_EX_TOPMOST grant no
    // exemption — the check is purely on geometry.
    // `ITaskbarList3::MarkFullscreenWindow(FALSE)` doesn't help
    // either because it targets the shell, not DWM. Shrinking by
    // 1 px is the standard defeat used by games / OBS / borderless
    // window helpers. The lost row/column is the bottom-right
    // pixel of the virtual screen, where no icon ever lives.
    //
    // Icon-position math is unaffected because it uses
    // `pt - bounds.left` / `pt - bounds.top`.
    //
    // Do NOT restore the exact-match.
    RECT {
        left: x,
        top: y,
        right: x + w - 1,
        bottom: y + h - 1,
    }
}

/// Place `overlay` in the Z-order so it sits **above** the desktop
/// (wallpaper + Shell icons) but **below** every ordinary application
/// window and every topmost window (taskbar, tooltips, notification
/// flyouts, start-menu overlay).
///
/// # Approach: walk from `Progman` upward
///
/// `HWND_BOTTOM` would sink below `Progman`/`WorkerW`, letting the
/// wallpaper cover the overlay. `HWND_NOTOPMOST` alone places it at
/// the *top* of the non-topmost band, above every application window.
/// Anchoring below `Shell_TrayWnd` is worse still: anchor calls to a
/// topmost window promote the caller to topmost.
///
/// Instead: locate `Progman` (always exists on stock Explorer), walk
/// upward through Z-order via `GetWindow(GW_HWNDPREV)` skipping every
/// `Progman` / `WorkerW` (all wallpaper-layer windows), and stop at
/// the first visible "normal" window. Insert the overlay just below
/// it — which is just above the wallpaper layer, and below every
/// currently-visible application window and the taskbar (topmost, so
/// above every non-topmost window by definition).
///
/// # Persistence
///
/// New application windows spawn at top of the non-topmost band —
/// above the overlay. `WS_EX_NOACTIVATE` keeps the overlay from
/// being bumped up by user clicks. If an application above it
/// closes mid-animation, the overlay stays put (just above the
/// wallpaper layer) — still correct.
///
/// # Fallback
///
/// If there is no visible non-desktop window (unusual — a stripped
/// test rig with only Explorer running), or `Progman` isn't findable
/// (custom shell), fall back to `HWND_NOTOPMOST` — top of non-
/// topmost band, still below the taskbar. Non-fatal on any error.
fn sink_overlay_below_taskbars(overlay: HWND) {
    // Demote out of the topmost band if a prior SetWindowPos promoted
    // the window. Idempotent when already non-topmost.
    //
    // SAFETY: `overlay` is this module's own HWND on the current STA
    // thread; `HWND_NOTOPMOST` is a documented sentinel. The SWP_NO*
    // flags restrict the call to the Z-order band.
    let _ = unsafe {
        SetWindowPos(
            overlay,
            Some(HWND_NOTOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    };

    if let Some(anchor) = find_lowest_normal_window() {
        // SAFETY: `anchor` was just returned by the Z-order walk and
        // is a valid visible non-desktop HWND. SetWindowPos on
        // (overlay, anchor) places overlay just below the anchor —
        // above `Progman` and `WorkerW`, below the anchor and every
        // window above it.
        let _ = unsafe {
            SetWindowPos(
                overlay,
                Some(anchor),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        };
    }
    // else: no anchor to sink under (no normal windows currently
    // visible, or `Progman` missing) — leave overlay at
    // `HWND_NOTOPMOST` position. Still below the taskbar; only
    // above any windows that appear later, which is fine because
    // no windows currently exist to be above.
}

/// Walk the Z-order upward starting from `Progman` and return the
/// lowest visible window whose class is *not* one of the desktop /
/// wallpaper family (`Progman`, `WorkerW`). Returns `None` if
/// `Progman` isn't findable or every window above it is still a
/// wallpaper-layer window.
///
/// # Why start from Progman
/// `Progman` is Explorer's own desktop container and always sits
/// near the bottom of the top-level Z-order, which makes it a
/// deterministic starting point without scanning every top-level
/// window in the system.
fn find_lowest_normal_window() -> Option<HWND> {
    // SAFETY: `FindWindowW` with a NUL-terminated class name and NULL
    // window title is a documented lookup. Result is either a valid
    // HWND or an error, treated as `None`.
    let progman = unsafe { FindWindowW(w!("Progman"), PCWSTR::null()) }.ok()?;
    if progman.is_invalid() {
        return None;
    }

    // Walk upward. `GW_HWNDPREV` returns "the window above the
    // specified window in the Z order". Bounded loop so a corrupt
    // Z-order list cannot hang the walk.
    let mut cursor = progman;
    for _ in 0..1024 {
        // SAFETY: `GetWindow` is safe on any HWND; returns an error
        // when there's no window in the requested relation.
        let next = match unsafe { GetWindow(cursor, GW_HWNDPREV) } {
            Ok(h) if !h.is_invalid() => h,
            _ => return None,
        };
        if !is_desktop_family_class(next) && is_window_visible(next) {
            return Some(next);
        }
        cursor = next;
    }
    None
}

/// Query the class name of `hwnd` and return `true` if it belongs to
/// Explorer's wallpaper / desktop-icons family. Used by the
/// `sink_overlay_below_taskbars` Z-order walk to skip over
/// wallpaper-layer windows.
fn is_desktop_family_class(hwnd: HWND) -> bool {
    let mut buf = [0u16; 64];
    // SAFETY: `GetClassNameW` writes at most `buf.len()` u16s and
    // returns the number of chars copied (excluding trailing NUL).
    // 0 means failure, treated as "not desktop family".
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        return false;
    }
    let name = &buf[..len as usize];
    // Both class names are ASCII, so a direct UTF-16 slice compare
    // is fine.
    const PROGMAN: &[u16] = &[b'P' as u16, b'r' as u16, b'o' as u16, b'g' as u16, b'm' as u16, b'a' as u16, b'n' as u16];
    const WORKERW: &[u16] = &[b'W' as u16, b'o' as u16, b'r' as u16, b'k' as u16, b'e' as u16, b'r' as u16, b'W' as u16];
    name == PROGMAN || name == WORKERW
}

fn is_window_visible(hwnd: HWND) -> bool {
    // SAFETY: `IsWindowVisible` is safe on any HWND value.
    unsafe { IsWindowVisible(hwnd) }.as_bool()
}

impl OverlaySession {
    /// Build a fresh overlay covering the virtual screen, load every
    /// icon's bitmap + label into GPU-side caches, and pre-render the
    /// first frame at each icon's `source_position`.
    pub(crate) fn new(plans: &[IconRenderPlan]) -> Result<Self, DesktopError> {
        // Taken first: everything below mutates process-global state
        // (the cancel atom, the window class, DPI awareness), so a
        // second concurrent session must be refused before any of it
        // is touched.
        let claim = OverlayClaim::acquire()?;
        ensure_dpi_awareness();
        register_window_class()?;
        // Clear any cancel reason that may have been signalled by a
        // prior session's WNDPROC but never consumed (e.g. because
        // the previous session ended with an error before its next
        // commit).
        reset_cancel_state();

        let bounds = virtual_screen_bounds();
        let width_px = (bounds.right - bounds.left).max(1);
        let height_px = (bounds.bottom - bounds.top).max(1);

        // ---- HWND -----------------------------------------------------
        let hinstance = unsafe { GetModuleHandleW(None) }
            .map_err(|e| overlay_err("GetModuleHandleW (create)", e))?;
        // SAFETY: standard CreateWindowExW; parent = None → top level.
        let hwnd = HwndGuard(
            unsafe {
                CreateWindowExW(
                    WS_EX_LAYERED
                        | WS_EX_NOREDIRECTIONBITMAP
                        | WS_EX_NOACTIVATE
                        | WS_EX_TRANSPARENT
                        | WS_EX_TOOLWINDOW,
                    OVERLAY_CLASS,
                    w!("RustyDesktopIcons Overlay"),
                    WS_POPUP,
                    bounds.left,
                    bounds.top,
                    width_px,
                    height_px,
                    None,
                    None,
                    Some(hinstance.into()),
                    None,
                )
            }
            .map_err(|e| overlay_err("CreateWindowExW", e))?,
        );

        // Sink the overlay just below the taskbar tier so the tray,
        // start button and clock stay visible during the animation.
        // See `sink_overlay_below_taskbars` for the
        // Windows-11-taskbar-not-topmost rationale.
        // Non-fatal if the taskbar HWNDs aren't findable — the
        // overlay stays where `CreateWindowExW` placed it.
        sink_overlay_below_taskbars(hwnd.0);

        // ---- D3D11 → DXGI → D2D → DComp chain ------------------------
        let mut d3d_device_opt: Option<ID3D11Device> = None;
        // SAFETY: standard D3D11CreateDevice usage.
        let hr = unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d_device_opt),
                None,
                None,
            )
        };
        hr.map_err(|e| overlay_err("D3D11CreateDevice", e))?;
        let d3d_device = d3d_device_opt.ok_or_else(|| {
            DesktopError::OverlayUnavailable(
                "D3D11CreateDevice returned S_OK but no device".into(),
            )
        })?;
        let dxgi_device: IDXGIDevice = d3d_device
            .cast()
            .map_err(|e| overlay_err("ID3D11Device → IDXGIDevice", e))?;
        let d2d_device: ID2D1Device = unsafe {
            D2D1CreateDevice(&dxgi_device, None)
                .map_err(|e| overlay_err("D2D1CreateDevice", e))?
        };
        let d2d_context: ID2D1DeviceContext = unsafe {
            d2d_device
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
                .map_err(|e| overlay_err("CreateDeviceContext", e))?
        };
        let dcomp_device: IDCompositionDevice = unsafe {
            DCompositionCreateDevice::<_, IDCompositionDevice>(&dxgi_device)
                .map_err(|e| overlay_err("DCompositionCreateDevice", e))?
        };
        let dcomp_target: IDCompositionTarget = unsafe {
            dcomp_device
                .CreateTargetForHwnd(hwnd.0, true)
                .map_err(|e| overlay_err("CreateTargetForHwnd", e))?
        };
        let root_visual: IDCompositionVisual = unsafe {
            dcomp_device
                .CreateVisual()
                .map_err(|e| overlay_err("CreateVisual", e))?
        };
        unsafe {
            dcomp_target
                .SetRoot(&root_visual)
                .map_err(|e| overlay_err("SetRoot", e))?;
        }

        // ---- Persistent DComp surface ------------
        let surface: IDCompositionSurface = unsafe {
            dcomp_device
                .CreateSurface(
                    width_px as u32,
                    height_px as u32,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_ALPHA_MODE_PREMULTIPLIED,
                )
                .map_err(|e| overlay_err("CreateSurface", e))?
        };
        unsafe {
            root_visual
                .SetContent(&surface)
                .map_err(|e| overlay_err("SetContent", e))?;
        }

        // ---- DirectWrite factory + system icon-title font ------------
        // SAFETY: GetDpiForWindow always succeeds; returns 96 if
        // window is DPI-unaware.
        let dpi = unsafe { GetDpiForWindow(hwnd.0) };
        let dpi_px = if dpi == 0 { 96 } else { dpi };
        let text = build_text_resources(dpi_px)?;

        // ---- Brushes --------------------------------------------------
        //
        // Label glyphs: opaque white; label shadow: 50 % black.
        // Selection tint and focus outline are intentionally not
        // rendered (see module doc).
        let placeholder_brush = create_brush(
            &d2d_context,
            D2D1_COLOR_F { r: 0.0, g: 0.7, b: 1.0, a: 0.85 },
            "placeholder_brush",
        )?;
        // Premultiplied white = (1, 1, 1, 1); no math needed.
        let label_brush = create_brush(
            &d2d_context,
            D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
            "label_brush",
        )?;
        // 50 % black premultiplied = (0, 0, 0, 0.5).
        let label_shadow_brush = create_brush(
            &d2d_context,
            D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.5 },
            "label_shadow_brush",
        )?;

        // ---- Per-icon caches ------------------------------------------
        //
        // Bitmap upload happens once per icon at session start. Even
        // with 100 icons at 48×48 this is <1 MB of pixel data.
        let mut icons: HashMap<IconId, IconEntry> = HashMap::with_capacity(plans.len());
        for plan in plans {
            let entry = build_icon_entry(&d2d_context, &text, plan);
            icons.insert(plan.id.clone(), entry);
        }

        let sprites = if plans.iter().any(|plan| plan.effect.is_some()) {
            Some(build_sprite_renderer(&d3d_device, &d2d_context, &icons, plans, &Brushes {
                placeholder: &placeholder_brush, label: &label_brush, label_shadow: &label_shadow_brush,
            })?)
        } else { None };

        let mut this = OverlaySession {
            // From here on `OverlaySession::Drop` owns the window.
            hwnd: hwnd.disarm(),
            sprites,
            background: plans.iter().filter(|plan| plan.stationary).map(|plan| (plan.id.clone(), plan.source_position)).collect(),
            visual_frame: plans.iter().map(|plan| rdi_core::IconFrame {
                id: plan.id.clone(), position: plan.source_position, progress: 0.0, elapsed_seconds: 0.0,
            }).collect(),
            _d3d_device: d3d_device,
            _dxgi_device: dxgi_device,
            _d2d_device: d2d_device,
            d2d_context,
            dcomp_device,
            _dcomp_target: dcomp_target,
            _root_visual: root_visual,
            surface,
            placeholder_brush,
            label_brush,
            label_shadow_brush,
            icons,
            text,
            bounds,
            shown: false,
            _claim: claim,
        };

        // ---- Pre-render first frame at source positions --------------
        let initial: Vec<(IconId, Point)> = plans
            .iter()
            .filter(|plan| !plan.stationary)
            .map(|p| (p.id.clone(), p.source_position))
            .collect();
        this.render_frame(&initial)?;
        // Commit the pre-render but keep the window hidden — the real
        // Shell icons are still visible.
        unsafe {
            this.dcomp_device
                .Commit()
                .map_err(|e| overlay_err("dcomp Commit (pre-render)", e))?;
        }

        Ok(this)
    }

    /// Render a single frame with the given icon positions.
    fn render_frame(&mut self, positions: &[(IconId, Point)]) -> Result<(), DesktopError> {
        // BeginDraw returns the current back-buffer as an IDXGISurface,
        // wrapped in a fresh ID2D1Bitmap1 each frame (cheap — a handle,
        // not an upload) and set as the D2D target.
        let (dxgi_surface, _offset) = unsafe {
            let mut off = POINT::default();
            let s: IDXGISurface = self
                .surface
                .BeginDraw(None, &mut off)
                .map_err(|e| overlay_err("BeginDraw", e))?;
            (s, off)
        };
        if let Some(sprites) = &mut self.sprites {
            let update = RECT {
                left: _offset.x, top: _offset.y,
                right: _offset.x.saturating_add(self.bounds.right - self.bounds.left),
                bottom: _offset.y.saturating_add(self.bounds.bottom - self.bounds.top),
            };
            let result = sprites.render_region(&dxgi_surface, [_offset.x as f32, _offset.y as f32],
                update, Point::new(self.bounds.left, self.bounds.top), &self.visual_frame, false);
            // SAFETY: this surface has an outstanding BeginDraw on this thread, including on render failure.
            let ended = unsafe { self.surface.EndDraw() }.map_err(|error| overlay_err("sprite EndDraw", error));
            return result.and(ended);
        }
        let bmp_props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        let target: ID2D1Bitmap1 = unsafe {
            self.d2d_context
                .CreateBitmapFromDxgiSurface(&dxgi_surface, Some(&bmp_props))
                .map_err(|e| overlay_err("CreateBitmapFromDxgiSurface", e))?
        };
        unsafe { self.d2d_context.SetTarget(&target) };

        // ---- Draw all icons -------------------------------------------
        unsafe {
            self.d2d_context.BeginDraw();
            self.d2d_context.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));

            for (id, pt) in positions.iter().chain(self.background.iter()) {
                let entry = match self.icons.get(id) {
                    Some(e) => e,
                    None => continue,
                };
                // Translate virtual-screen coords → overlay-local
                // coords.
                let x = (pt.x - self.bounds.left) as f32;
                let y = (pt.y - self.bounds.top) as f32;
                let brushes = Brushes {
                    placeholder: &self.placeholder_brush,
                    label: &self.label_brush,
                    label_shadow: &self.label_shadow_brush,
                };
                draw_icon(&self.d2d_context, &brushes, entry, x, y, None);
            }

            let mut tag1 = 0u64;
            let mut tag2 = 0u64;
            self.d2d_context
                .EndDraw(Some(&mut tag1), Some(&mut tag2))
                .map_err(|e| overlay_err("EndDraw", e))?;

            self.surface
                .EndDraw()
                .map_err(|e| overlay_err("surface EndDraw", e))?;
        }
        Ok(())
    }

    /// Push a new frame of icon positions. On the first call, also
    /// shows the overlay window.
    pub(crate) fn commit_frame(
        &mut self,
        positions: &[(IconId, Point)],
    ) -> Result<(), DesktopError> {
        self.render_frame(positions)?;
        unsafe {
            self.dcomp_device
                .Commit()
                .map_err(|e| overlay_err("dcomp Commit", e))?;
        }
        if !self.shown {
            // SAFETY: the live composition device has just accepted its initial frame.
            unsafe { self.dcomp_device.WaitForCommitCompletion() }
                .map_err(|error| overlay_err("initial composition wait", error))?;
            let _ = unsafe { ShowWindow(self.hwnd, SW_SHOWNOACTIVATE) };
            self.shown = true;
        }
        Ok(())
    }

    pub(crate) fn set_visual_frame(&mut self, frame: &[rdi_core::IconFrame]) {
        self.visual_frame.clear();
        self.visual_frame.extend_from_slice(frame);
    }

    pub(crate) fn capture(&mut self, seconds: f64) -> Result<rdi_core::CapturedFrame, DesktopError> {
        let width = (self.bounds.right - self.bounds.left) as u32;
        let height = (self.bounds.bottom - self.bounds.top) as u32;
        rdi_core::Canvas { width, height, dpi_scale: 1.0, icon_size: 48 }.validate()?;
        let target = crate::sprites::CaptureTarget::new(&self._d3d_device, width, height)?;
        let mut frame = self.visual_frame.clone();
        frame.extend(self.background.iter().map(|(id, position)| rdi_core::IconFrame {
            id: id.clone(), position: *position, progress: 0.0, elapsed_seconds: 0.0,
        }));
        render_capture(&target, width, height, Point::new(self.bounds.left, self.bounds.top),
            &self.d2d_context, &self.icons, &Brushes { placeholder: &self.placeholder_brush,
                label: &self.label_brush, label_shadow: &self.label_shadow_brush }, &mut self.sprites, &frame)?;
        target.read(seconds)
    }

    pub(crate) fn clean_frame(&mut self, positions: &[(IconId, Point)]) -> Result<(), DesktopError> {
        self.visual_frame.clear();
        self.visual_frame.extend(positions.iter().map(|(id, position)| rdi_core::IconFrame {
            id: id.clone(), position: *position, progress: 1.0, elapsed_seconds: 0.0,
        }));
        self.commit_frame(positions)
    }

    /// Hide and destroy the overlay window.
    pub(crate) fn shutdown(&mut self) {
        if self.hwnd.0.is_null() {
            return;
        }
        // Force the D3D11 runtime to release the device's internal
        // resource refs. Without this, back-to-back overlay sessions
        // (long-running `overlay_wiggle` loops, aggressive
        // enrich→animate churn) eventually exhaust the driver
        // allocator and `D3D11CreateDevice` returns E_OUTOFMEMORY
        // after a few hundred cycles.
        //
        // SAFETY: called on the STA thread that constructed the
        // device; `GetImmediateContext` is safe on any valid device
        // and `ClearState` / `Flush` take no args.
        unsafe {
            if let Ok(ctx) = self._d3d_device.GetImmediateContext() {
                let ctx: ID3D11DeviceContext = ctx;
                ctx.ClearState();
                ctx.Flush();
            }
        }
        // Let DirectComposition drop its side of the visual-tree
        // references synchronously — otherwise DWM can hold the last
        // surface until the next composition tick, which stacks up
        // across rapid begin/finalize cycles.
        //
        // SAFETY: `_dcomp_target` and `dcomp_device` are alive here
        // (Rust drops fields only after `Drop::drop` returns).
        unsafe {
            let _ = self._dcomp_target.SetRoot(None);
            let _ = self.dcomp_device.Commit();
        }
        // SAFETY: destroying an own HWND on the STA thread that
        // created it is always safe.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
        self.hwnd = HWND::default();
    }
}

impl Drop for OverlaySession {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn create_brush(
    ctx: &ID2D1DeviceContext,
    color: D2D1_COLOR_F,
    label: &str,
) -> Result<ID2D1SolidColorBrush, DesktopError> {
    unsafe {
        ctx.CreateSolidColorBrush(&color, None)
            .map_err(|e| overlay_err(&format!("CreateSolidColorBrush ({label})"), e))
    }
}

fn atlas_layout(count: usize, cell_width: u32, cell_height: u32) -> Option<(u32, u32, u32)> {
    if cell_width == 0 || cell_height == 0 || cell_width > 8192 || cell_height > 8192 {
        return None;
    }
    let count = u32::try_from(count.max(1)).ok()?;
    let max_columns = 8192 / cell_width;
    let max_rows = 8192 / cell_height;
    if count > max_columns * max_rows {
        return None;
    }
    (1..=max_columns.min(count))
        .filter_map(|columns| {
            let rows = count.div_ceil(columns);
            (rows <= max_rows).then_some((columns, columns * cell_width, rows * cell_height))
        })
        .min_by_key(|&(columns, width, height)| (width * height, width.abs_diff(height), columns))
}

fn build_sprite_renderer(
    device: &ID3D11Device,
    context: &ID2D1DeviceContext,
    icons: &HashMap<IconId, IconEntry>,
    plans: &[IconRenderPlan],
    brushes: &Brushes<'_>,
) -> Result<crate::sprites::SpriteRenderer, DesktopError> {
    let margin = plans.iter().filter_map(|plan| plan.effect.as_ref().map(|effect| effect.padding_px)).max().unwrap_or(0) + 16;
    let cell_width = icons.values().map(|entry| entry.size_dip.0.max(entry.label_size_dip.0).ceil() as u32 + 2 * margin).max().unwrap_or(64);
    let cell_height = icons.values().map(|entry| (entry.size_dip.1 + entry.label_size_dip.1 + LABEL_GAP_DIP + entry.render_offset_dip.1.abs()).ceil() as u32 + 2 * margin).max().unwrap_or(64);
    let layer_count: usize = plans.iter().filter_map(|plan| plan.effect.as_ref()?.shader.execution.as_ref())
        .map(|execution| execution.body_artwork as usize + execution.label_artwork as usize).sum();
    let Some((columns, width, height)) = atlas_layout(plans.len() + layer_count, cell_width, cell_height) else {
        return Err(DesktopError::InvalidEffect("prepared artwork exceeds 8192px atlas limit; reduce padding or icon count".into()));
    };
    let atlas = crate::sprites::texture(device, width, height)?;
    let surface: IDXGISurface = atlas.cast().map_err(crate::sprites::gpu_error)?;
    let properties = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: 96.0, dpiY: 96.0, bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
        ..Default::default()
    };
    let mut sprites = Vec::with_capacity(plans.len());
    let mut layers = Vec::with_capacity(plans.len());
    let mut next_layer = plans.len();
    // SAFETY: atlas and context share the D3D device; every draw stays within its allocated cell.
    unsafe {
        let bitmap = context.CreateBitmapFromDxgiSurface(&surface, Some(&properties)).map_err(crate::sprites::gpu_error)?;
        context.SetTarget(&bitmap);
        context.BeginDraw();
        context.Clear(Some(&D2D1_COLOR_F::default()));
        for (index, plan) in plans.iter().enumerate() {
            let entry = &icons[&plan.id];
            let left = (index as u32 % columns * cell_width) as f32;
            let top = (index as u32 / columns * cell_height) as f32;
            let anchor = [((cell_width as f32 - entry.size_dip.0) * 0.5).floor(), margin as f32];
            context.PushAxisAlignedClip(&D2D_RECT_F { left, top, right: left + cell_width as f32, bottom: top + cell_height as f32 },
                windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED);
            draw_icon(context, brushes, entry, left + anchor[0], top + anchor[1], None);
            context.PopAxisAlignedClip();
            let mut regions = [[0.0; 4]; 2];
            if let Some(execution) = plan.effect.as_ref().and_then(|effect| effect.shader.execution.as_ref()) {
                for ((region, layer), required) in regions.iter_mut().zip([ArtworkLayer::Body, ArtworkLayer::Label])
                    .zip([execution.body_artwork, execution.label_artwork]) {
                    if !required { continue; }
                    let layer_left = (next_layer as u32 % columns * cell_width) as f32;
                    let layer_top = (next_layer as u32 / columns * cell_height) as f32;
                    *region = [layer_left, layer_top, cell_width as f32, cell_height as f32];
                    next_layer += 1;
                    context.PushAxisAlignedClip(&D2D_RECT_F { left: layer_left, top: layer_top,
                        right: layer_left + cell_width as f32, bottom: layer_top + cell_height as f32 },
                        windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED);
                    draw_icon_layer(context, brushes, entry, layer_left + anchor[0], layer_top + anchor[1], None, layer);
                    context.PopAxisAlignedClip();
                }
            }
            layers.push(regions);
            sprites.push(crate::sprites::Sprite {
                id: plan.id.clone(), origin: plan.source_position,
                destination: plan.final_position,
                region: [left, top, cell_width as f32, cell_height as f32], anchor, effect: plan.effect.clone(),
                geometry: [anchor[0] + entry.size_dip.0 * 0.5 + entry.render_offset_dip.0,
                    anchor[1] + entry.size_dip.1 * 0.5 + entry.render_offset_dip.1,
                    entry.size_dip.0, entry.size_dip.1],
            });
        }
        let ended = context.EndDraw(None, None);
        context.SetTarget(None);
        ended.map_err(crate::sprites::gpu_error)?;
    }
    crate::sprites::SpriteRenderer::new_layered(device, &atlas, sprites, layers)
}

#[cfg(test)]
mod sprite_tests {
    use super::*;

    #[test]
    #[ignore = "hardware off-screen capture"]
    fn gpu_silk_flow_layers_travel_and_endpoints() {
        use rdi_core::{AnimationCurve, Curve, Effect, IconFrame, Keyframe, KeyframeInterp, SceneRenderer as _};
        let canvas = rdi_core::Canvas { width: 640, height: 320, dpi_scale: 1.0, icon_size: 64 };
        let mut plan = IconRenderPlan::placeholder("silk".into(), Point::new(70, 110), Point::new(500, 110));
        let mut pixels = Vec::new();
        for row in 0..64 {
            for column in 0..64 {
                pixels.extend_from_slice(if column < 32 { &[240, 90, 45, 255] }
                    else if row < 32 { &[160, 65, 230, 255] } else { &[55, 190, 245, 255] });
            }
        }
        plan.size_px = (64, 64);
        plan.image = Some(rdi_core::IconBitmap { width: 64, height: 64, stride: 256, pixels });
        plan.label = Some(rdi_core::IconLabel { text: "SILK LABEL".into(), bounds_px: (120, 50) });
        let program = crate::shader::compile(crate::shader::BuiltinShader::SilkFlow).unwrap();
        let constant = |value| Curve::keyframes(vec![Keyframe::new(0.0, value), Keyframe::new(1.0, value)], KeyframeInterp::Linear).unwrap();
        let same_pixels = |actual: &[u8], expected: &[u8], phase: &str| {
            let differences = actual.iter().zip(expected).filter(|(actual, expected)| actual != expected).count();
            let maximum = actual.iter().zip(expected).map(|(actual, expected)| actual.abs_diff(*expected)).max().unwrap_or(0);
            assert!(actual == expected, "{phase}: {differences} differing bytes, maximum delta {maximum}");
        };
        plan.effect = Some(Effect { params: program.default_params(), shader: program,
            padding_px: 24, envelope: constant(1.0), seed: 7.0 });
        let make_frame = |plan: &IconRenderPlan, progress: f32| {
            let movement = Curve::ease_in_out().eval(progress);
            vec![IconFrame { id: plan.id.clone(), progress, elapsed_seconds: progress * 4.0,
                position: Point::new(
                    (plan.source_position.x as f32 + (plan.final_position.x - plan.source_position.x) as f32 * movement).round() as i32,
                    (plan.source_position.y as f32 + (plan.final_position.y - plan.source_position.y) as f32 * movement).round() as i32) }]
        };
        let mut renderer = SceneRenderer::new(canvas, &[plan.clone()]).unwrap();
        for progress in [0.0, 1.0] {
            let frame = make_frame(&plan, progress);
            let expected = render_snapshot(canvas.width, canvas.height, 1.0, &[plan.clone()],
                &[(plan.id.clone(), frame[0].position)]).unwrap().0;
            same_pixels(&renderer.render(&frame, progress as f64).unwrap().pixels, &expected, "endpoint");
        }
        let middle = make_frame(&plan, 0.5);
        let flow = renderer.render(&middle, 2.0).unwrap().pixels;
        assert!(flow.chunks_exact(4).all(|pixel| pixel[..3].iter().all(|channel| *channel <= pixel[3])));
        for start in [150, 260, 370, 450] {
            let count = flow.chunks_exact(4).enumerate().filter(|(index, pixel)| {
                let column = index % canvas.width as usize;
                (start..start + 50).contains(&column) && pixel[3] > 5
            }).count();
            assert!(count > 50, "missing continuous flow at {start}: {count}");
        }
        renderer.render(&make_frame(&plan, 0.9), 3.6).unwrap();
        same_pixels(&renderer.render(&middle, 2.0).unwrap().pixels, &flow, "rewind");
        let mut body_plan = plan.clone();
        body_plan.label = None;
        let mut body = SceneRenderer::new(canvas, &[body_plan]).unwrap();
        for progress in [0.25, 0.5, 0.85, 0.90] {
            let frame = make_frame(&plan, progress);
            let labeled = renderer.render(&frame, progress as f64).unwrap().pixels;
            let unlabeled = body.render(&frame, progress as f64).unwrap().pixels;
            assert!(labeled.iter().zip(&unlabeled).all(|(left, right)| left.abs_diff(*right) <= 1),
                "label visible at {progress}");
        }
        let arriving = make_frame(&plan, 0.97);
        assert_ne!(renderer.render(&arriving, 3.88).unwrap().pixels, body.render(&arriving, 3.88).unwrap().pixels);
        let mut alternate = middle.clone();
        alternate[0].position = Point::new(100, 40);
        same_pixels(&renderer.render(&alternate, 2.0).unwrap().pixels, &flow, "movement independence");
        let mut zero = plan.clone();
        zero.effect.as_mut().unwrap().envelope = constant(0.0);
        let mut zero_renderer = SceneRenderer::new(canvas, &[zero.clone()]).unwrap();
        same_pixels(&zero_renderer.render(&middle, 2.0).unwrap().pixels,
            &render_snapshot(canvas.width, canvas.height, 1.0, &[zero], &[(plan.id.clone(), middle[0].position)]).unwrap().0, "zero strength");
        let mut seeded = plan.clone();
        seeded.effect.as_mut().unwrap().seed = 19.0;
        assert_ne!(SceneRenderer::new(canvas, &[seeded]).unwrap().render(&middle, 2.0).unwrap().pixels, flow);
        for target in [Point::new(500, 190), plan.source_position, Point::new(0, 20)] {
            let mut directional = plan.clone();
            directional.final_position = target;
            let mut renderer = SceneRenderer::new(canvas, &[directional.clone()]).unwrap();
            let frame = make_frame(&directional, 0.5);
            let first = renderer.render(&frame, 2.0).unwrap().pixels;
            assert!(first.chunks_exact(4).any(|pixel| pixel[3] > 10));
            assert_eq!(renderer.render(&frame, 2.0).unwrap().pixels, first);
        }
    }

    #[test]
    #[ignore = "hardware off-screen capture"]
    fn gpu_scene_capture_rewinds_and_matches_snapshot() {
        use rdi_core::SceneRenderer as _;
        let canvas = rdi_core::Canvas { width: 320, height: 240, dpi_scale: 1.0, icon_size: 48 };
        let mut plan = IconRenderPlan::placeholder("capture".into(), Point::new(40, 40), Point::new(180, 60));
        plan.image = Some(rdi_core::IconBitmap { width: 48, height: 48, stride: 192, pixels: [40, 80, 120, 160].repeat(48 * 48) });
        plan.label = Some(rdi_core::IconLabel { text: "Capture".into(), bounds_px: (100, 60) });
        let frame = vec![rdi_core::IconFrame { id: plan.id.clone(), position: plan.source_position, progress: 0.0, elapsed_seconds: 0.0 }];
        let baseline = render_snapshot(320, 240, 1.0, &[plan.clone()], &[(plan.id.clone(), plan.source_position)]).unwrap().0;
        let mut plain = SceneRenderer::new(canvas, &[plan.clone()]).unwrap();
        assert_eq!(plain.render(&frame, 0.0).unwrap().pixels, baseline);
        for shader in crate::shader::BuiltinShader::ALL {
            let program = crate::shader::compile(*shader).unwrap();
            plan.effect = Some(rdi_core::Effect { params: program.default_params(), shader: program, padding_px: 24,
                envelope: rdi_core::Curve::linear(), seed: 7.0 });
            let mut renderer = SceneRenderer::new(canvas, &[plan.clone()]).unwrap();
            let start = renderer.render(&frame, 0.0).unwrap();
            assert!(start.pixels.iter().any(|value| *value != 0));
            let middle = vec![rdi_core::IconFrame { id: plan.id.clone(), position: Point::new(110, 50), progress: 0.5, elapsed_seconds: 0.5 }];
            let captured = renderer.render(&middle, 0.5).unwrap();
            renderer.render(&frame, 0.0).unwrap();
            assert_eq!(renderer.render(&middle, 0.5).unwrap().pixels, captured.pixels);
            assert_eq!(captured.pixels.len(), 320 * 240 * 4);
            assert!(captured.pixels.chunks_exact(4).all(|pixel| pixel[..3].iter().all(|channel| *channel <= pixel[3])));
        }
    }

    #[test]
    fn atlas_layout_fits_tall_and_wide_cells() {
        assert_eq!(atlas_layout(256, 254, 540), Some((32, 8128, 4320)));
        assert_eq!(atlas_layout(256, 540, 254), Some((8, 4320, 8128)));
        assert_eq!(atlas_layout(256, 128, 128), Some((16, 2048, 2048)));
        assert_eq!(atlas_layout(0, 64, 64), Some((1, 64, 64)));
        assert_eq!(atlas_layout(1, 8192, 8192), Some((1, 8192, 8192)));
        assert_eq!(atlas_layout(4, 4096, 4096), Some((2, 8192, 8192)));
    }

    #[test]
    fn atlas_layout_rejects_invalid_or_impossible_inputs() {
        for (count, width, height) in [
            (1, 0, 64), (1, 64, 0), (1, 8193, 1), (1, 1, 8193),
            (5, 4096, 4096), (usize::MAX, 1, 1), (1, u32::MAX, u32::MAX),
        ] {
            assert_eq!(atlas_layout(count, width, height), None);
        }
    }

    #[test]
    fn atlas_layout_matches_uniform_cell_capacity() {
        for cell_width in [1, 254, 540, 4096, 8192] {
            for cell_height in [1, 254, 540, 4096, 8192] {
                let capacity = (8192 / cell_width) * (8192 / cell_height);
                for count in [1, 17, 256, capacity, capacity + 1] {
                    let layout = atlas_layout(count as usize, cell_width, cell_height);
                    assert_eq!(layout.is_some(), count <= capacity);
                    if let Some((columns, width, height)) = layout {
                        assert!(width <= 8192 && height <= 8192);
                        assert_eq!(width, columns * cell_width);
                        assert!(columns * (height / cell_height) >= count);
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "hardware D2D/D3D artwork readback"]
    fn gpu_baked_artwork_matches_snapshot_and_shades_decorations() {
        let device = crate::sprites::tests::device();
        let dxgi: IDXGIDevice = device.cast().unwrap();
        // SAFETY: both contexts share a valid device on this test thread.
        let context = unsafe {
            D2D1CreateDevice(&dxgi, None).unwrap()
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE).unwrap()
        };
        let text = build_text_resources(96).unwrap();
        let color = D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
        let placeholder = create_brush(&context, D2D1_COLOR_F { r: 0.0, g: 0.7, b: 1.0, a: 0.85 }, "test").unwrap();
        let label = create_brush(&context, color, "test").unwrap();
        let shadow = create_brush(&context, D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.5 }, "test").unwrap();
        let brushes = Brushes { placeholder: &placeholder, label: &label, label_shadow: &shadow };
        let mut plan = IconRenderPlan::placeholder(IconId::from("artwork"), Point::new(96, 64), Point::new(96, 64));
        plan.label = Some(rdi_core::IconLabel { text: "Label with badge".into(), bounds_px: (120, 60) });
        plan.shortcut_arrow_image = Some(rdi_core::IconBitmap {
            width: 16, height: 16, stride: 64, pixels: [0u8, 255, 0, 255].repeat(256),
        });
        plan.effect = Some(rdi_core::Effect {
            shader: crate::shader::compile("float4 pixel(VertexOutput input) : SV_Target { return 0; }").unwrap(),
            params: [0.0; 4], padding_px: 16, envelope: rdi_core::Curve::linear(), seed: 0.0,
        });
        let positions = [(plan.id.clone(), plan.source_position)];
        let (baseline, geometry) = render_snapshot(256, 256, 1.0, &[plan.clone()], &positions).unwrap();
        assert!(geometry[0].label_rect_px.is_some());
        assert!(geometry[0].arrow_rect_px.is_some());
        let icons = HashMap::from([(plan.id.clone(), build_icon_entry(&context, &text, &plan))]);
        let mut renderer = build_sprite_renderer(&device, &context, &icons, &[plan.clone()], &brushes).unwrap();
        let target = crate::sprites::texture(&device, 256, 256).unwrap();
        let surface: IDXGISurface = target.cast().unwrap();
        let mut frame = [rdi_core::IconFrame { id: plan.id.clone(), position: plan.source_position, progress: 0.0, elapsed_seconds: 0.0 }];
        renderer.render(&surface, [0.0; 2], Point::new(0, 0), &frame, false).unwrap();
        let actual = crate::sprites::tests::read_pixels(&device, &target);
        assert!(actual.chunks_exact(4).filter(|pixel| pixel[3] > 0).count() > 2500);
        let differing = actual.iter().zip(&baseline).filter(|(left, right)| left.abs_diff(**right) > 2).count();
        assert!(differing < 100, "identity artwork differs in {differing} channels");
        frame[0].progress = 0.999999;
        renderer.render(&surface, [0.0; 2], Point::new(0, 0), &frame, false).unwrap();
        assert!(crate::sprites::tests::read_pixels(&device, &target).iter().all(|value| *value == 0));
        let program = crate::shader::compile(crate::shader::BuiltinShader::ParticleVortex).unwrap();
        plan.effect = Some(rdi_core::Effect {
            params: program.default_params(), shader: program, padding_px: 16,
            envelope: rdi_core::Curve::keyframes(vec![
                rdi_core::Keyframe::new(0.0, 1.0), rdi_core::Keyframe::new(1.0, 1.0),
            ], rdi_core::KeyframeInterp::Linear).unwrap(), seed: 13.0,
        });
        let mut renderer = build_sprite_renderer(&device, &context, &icons, &[plan], &brushes).unwrap();
        for progress in [0.0, 1.0] {
            frame[0].progress = progress;
            renderer.render(&surface, [0.0; 2], Point::new(0, 0), &frame, false).unwrap();
            assert_eq!(crate::sprites::tests::read_pixels(&device, &target), actual);
        }
        frame[0].progress = 0.5;
        renderer.render(&surface, [0.0; 2], Point::new(0, 0), &frame, false).unwrap();
        let dust = crate::sprites::tests::read_pixels(&device, &target);
        assert_ne!(dust, actual);
        assert!(dust.chunks_exact(4).any(|pixel| pixel[3] > 30 && pixel[1] > 30 && pixel[0] < 3 && pixel[2] < 3), "green badge particles missing");
        assert!(dust.chunks_exact(4).any(|pixel| pixel[3] > 30 && pixel[0] > 30 && pixel[0].abs_diff(pixel[1]) < 3 && pixel[0].abs_diff(pixel[2]) < 3), "white label particles missing");
    }
}

fn build_icon_entry(
    ctx: &ID2D1DeviceContext,
    text: &TextResources,
    plan: &IconRenderPlan,
) -> IconEntry {
    let size_dip = match plan.size_px {
        (w, h) if w > 0 && h > 0 => (w as f32, h as f32),
        _ => (FALLBACK_ICON_SIZE_DIP, FALLBACK_ICON_SIZE_DIP),
    };

    // Intrinsic bitmap dimensions — populated from the actual
    // returned bitmap when there is one, else falls back to the
    // slot size (so placeholder-only icons render at their slot
    // dimensions unchanged).
    let intrinsic_size_dip = plan
        .image
        .as_ref()
        .map(|img| (img.width.max(1) as f32, img.height.max(1) as f32))
        .unwrap_or(size_dip);

    let bitmap = plan
        .image
        .as_ref()
        .and_then(|img| upload_icon_bitmap(ctx, img, &plan.id, "image"));

    let shortcut_arrow_bitmap = plan
        .shortcut_arrow_image
        .as_ref()
        .and_then(|img| upload_icon_bitmap(ctx, img, &plan.id, "shortcut_arrow_image"));

    // Build the label layout once. Bounds come from `IconLabel.bounds_px`
    // (in DIPs — treats "px" and "DIPs" as interchangeable
    // because the D2D bitmap DPI is fixed at 96).
    let (label_layout, label_size_dip) = match &plan.label {
        Some(label) => {
            let (bw, bh) = label.bounds_px;
            let bw = bw as f32;
            let bh = bh as f32;
            let layout = build_label_layout(text, &label.text, bw, bh).ok();
            (layout, (bw, bh))
        }
        None => (None, (0.0, 0.0)),
    };

    IconEntry {
        bitmap,
        size_dip,
        intrinsic_size_dip,
        label_layout,
        label_size_dip,
        render_offset_dip: (plan.render_offset_px.0 as f32, plan.render_offset_px.1 as f32),
        shortcut_arrow_bitmap,
    }
}

/// Upload a premultiplied-BGRA [`IconBitmap`] to a device-side
/// [`ID2D1Bitmap`]. `field_label` is used only in the failure
/// message so `image` and `shortcut_arrow_image` failures can be
/// distinguished in logs.
fn upload_icon_bitmap(
    ctx: &ID2D1DeviceContext,
    img: &rdi_core::IconBitmap,
    icon_id: &IconId,
    field_label: &str,
) -> Option<ID2D1Bitmap> {
    let props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: windows::Win32::Graphics::Direct2D::D2D1_BITMAP_OPTIONS_NONE,
        colorContext: std::mem::ManuallyDrop::new(None),
    };
    let size = D2D_SIZE_U {
        width: img.width,
        height: img.height,
    };
    // SAFETY: `img.pixels` outlives the `CreateBitmap` call — D2D
    // copies the source pixels into a fresh device bitmap. The
    // pointer / stride are valid for `size.height * stride` bytes,
    // which matches the buffer length asserted at construction.
    let result: windows::core::Result<ID2D1Bitmap1> = unsafe {
        ctx.CreateBitmap(
            size,
            Some(img.pixels.as_ptr() as *const _),
            img.stride,
            &props,
        )
    };
    match result {
        Ok(b) => b.cast::<ID2D1Bitmap>().ok(),
        Err(e) => {
            tracing::warn!(
                field = field_label,
                icon_id = %icon_id,
                hresult = format_args!("0x{:08X}", e.code().0 as u32),
                message = %e.message(),
                "overlay bitmap upload failed"
            );
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Icon draw helper (shared by live overlay + snapshot renderer)
// ---------------------------------------------------------------------------

/// Bundle of the three brushes `draw_icon` reads. Live path stores the
/// brushes on the `OverlaySession`; snapshot path builds fresh brushes
/// per call. Both borrow them into this bundle immediately before
/// calling `draw_icon`.
pub(crate) struct Brushes<'a> {
    pub placeholder: &'a ID2D1SolidColorBrush,
    pub label: &'a ID2D1SolidColorBrush,
    pub label_shadow: &'a ID2D1SolidColorBrush,
}

/// Return the DIP size Explorer paints the shortcut arrow at for a
/// slot whose DIP width is `slot_w`. Arrows are square, so callers
/// use the result in both axes. The three cutoffs (17 / 31 / 32)
/// and the two transitions (72 / 108) are empirical from a sweep of
/// 180 shortcut icons across 59 resolution × scale combinations — see
/// [temp_python/analyze_shortcut_arrow_ratio.py](../../../../temp_python/analyze_shortcut_arrow_ratio.py).
fn shortcut_arrow_target_size_dip(slot_w: f32) -> f32 {
    if slot_w <= 72.0 {
        17.0
    } else if slot_w <= 108.0 {
        31.0
    } else {
        32.0
    }
}

/// Draw a single icon at overlay-local coordinates `(x, y)`.
///
/// Free function so both the live `OverlaySession::render_frame` and
/// the headless [`render_snapshot`] path can call it without
/// duplicating draw code. Draw order (bottom → top):
///   1. Icon bitmap (or placeholder rounded rectangle).
///   2. Label shadow (+1 DIP offset).
///   3. Label glyphs.
///
/// Selection tints and focus outlines are intentionally not drawn —
/// see the module doc.
///
/// When `out_geom` is `Some`, the icon-bitmap draw rect and (if the
/// icon has a label) the DirectWrite `GetMetrics`-derived label rect
/// are written to it — both in overlay-local DIP coordinates
/// (identical to physical pixels since the D2D context is fixed at
/// 96 DPI).
///
/// # Safety
/// Caller must have opened a BeginDraw/EndDraw pair on `ctx`.
pub(crate) unsafe fn draw_icon(
    ctx: &ID2D1DeviceContext,
    brushes: &Brushes<'_>,
    entry: &IconEntry,
    x: f32,
    y: f32,
    out_geom: Option<&mut IconPaintGeometry>,
) {
    // SAFETY: forwarded context and entry obey the caller's active BeginDraw/EndDraw contract.
    unsafe { draw_icon_layer(ctx, brushes, entry, x, y, out_geom, ArtworkLayer::Complete) };
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArtworkLayer {
    Complete,
    Body,
    Label,
}

unsafe fn draw_icon_layer(
    ctx: &ID2D1DeviceContext,
    brushes: &Brushes<'_>,
    entry: &IconEntry,
    x: f32,
    y: f32,
    out_geom: Option<&mut IconPaintGeometry>,
    layer: ArtworkLayer,
) {
    // Apply per-icon render offset.
    let (dx, dy) = entry.render_offset_dip;
    let x = x + dx;
    let y = y + dy;
    let (w, h) = entry.size_dip;

    // Aspect-fit the bitmap into the slot without upscaling.
    // Explorer paints thumbnails at their intrinsic size when the
    // requested slot is larger than the returned bitmap; stretching
    // to fill the slot instead shows as a Δ = [-13, -28, 30, 28] px
    // mismatch for image shortcuts at 200 % DPI.
    let (iw, ih) = entry.intrinsic_size_dip;
    let (fit_w, fit_h) = if iw <= 0.0 || ih <= 0.0 || w <= 0.0 || h <= 0.0 {
        (w, h)
    } else {
        // Preserve aspect ratio. Never upscale past intrinsic in
        // either axis: `scale = min(1, min(w/iw, h/ih))`.
        let scale = (w / iw).min(h / ih).min(1.0);
        (iw * scale, ih * scale)
    };
    // Flat 2 px shadow reserve for thumbnails — Explorer draws a
    // soft drop shadow bottom-right of every thumbnail; the overlay
    // doesn't draw the shadow, but subtracting a fixed 2 px from
    // both `icon_left` and `icon_top` keeps the thumbnail where
    // Explorer would place it without the shadow. DPI-independent
    // per live measurement.
    let shadow_reserve_dip = if fit_h < h {
        THUMBNAIL_SHADOW_RESERVE_DIP
    } else {
        0.0
    };
    let icon_left = x + (w - fit_w) * 0.5 - shadow_reserve_dip;
    // Thumbnails narrower than the slot in the vertical axis are
    // bottom-anchored inside the slot with a DPI-scaled inset —
    // matches Explorer's paint (~2 px @ 100 %, 5 px @ 200 %,
    // 6 px @ 250 %, 10 px @ 400 %) — plus the flat shadow reserve
    // above. Slot-filling icons (`fit_h == h`) keep the plain `y`
    // origin.
    let icon_top = if fit_h < h {
        let bottom_inset_dip = (h / THUMBNAIL_BOTTOM_INSET_DIVISOR)
            .round()
            .max(THUMBNAIL_BOTTOM_INSET_MIN_DIP);
        y + (h - fit_h) - bottom_inset_dip - shadow_reserve_dip
    } else {
        y
    };

    // 1. Icon bitmap (or placeholder tile when there is none).
    let icon_rect = D2D_RECT_F {
        left: icon_left,
        top: icon_top,
        right: icon_left + fit_w,
        bottom: icon_top + fit_h,
    };
    if layer != ArtworkLayer::Label {
    match &entry.bitmap {
        Some(bmp) => unsafe {
            ctx.DrawBitmap(
                bmp,
                Some(&icon_rect),
                1.0,
                windows::Win32::Graphics::Direct2D::D2D1_INTERPOLATION_MODE_LINEAR,
                None,
                None,
            );
        },
        None => {
            let rr = D2D1_ROUNDED_RECT {
                rect: icon_rect,
                radiusX: CORNER_RADIUS_DIP,
                radiusY: CORNER_RADIUS_DIP,
            };
            unsafe { ctx.FillRoundedRectangle(&rr, brushes.placeholder) };
        }
    }
    }

    // 1a. Shortcut-arrow overlay — anchored to the actual
    // thumbnail's bottom-left, mirrored by `shadow_reserve_dip`
    // (down-left) so the arrow lives in the shadow gutter Explorer
    // paints below-left of thumbnailed items. For slot-filling
    // icons (`fit_h == h`, `fit_w == w`) `shadow_reserve_dip` is 0
    // and `icon_left == x`, so the anchor is the slot bottom-left.
    //
    // The arrow's *size* is a step function of the slot's DIP
    // width (17 / 31 / 32 DIP at the three empirical cutoffs) —
    // see [`shortcut_arrow_target_size_dip`].
    let arrow_rect_dip = entry.shortcut_arrow_bitmap.as_ref().filter(|_| layer != ArtworkLayer::Label).and_then(|arrow_bmp| {
        let target = shortcut_arrow_target_size_dip(fit_w);
        let draw_w = target.min(w).max(1.0);
        let draw_h = target.min(h).max(1.0);
        let arrow_left = icon_left - shadow_reserve_dip;
        let arrow_top = y + h - draw_h;
        let arrow_rect = D2D_RECT_F {
            left: arrow_left,
            top: arrow_top,
            right: arrow_left + draw_w,
            bottom: arrow_top + draw_h,
        };
        unsafe {
            ctx.DrawBitmap(
                arrow_bmp,
                Some(&arrow_rect),
                1.0,
                windows::Win32::Graphics::Direct2D::D2D1_INTERPOLATION_MODE_LINEAR,
                None,
                None,
            );
        }
        Some((arrow_left, arrow_top, draw_w, draw_h))
    });

    // 2 + 3. Label with drop shadow.
    if let Some(layout) = entry.label_layout.as_ref().filter(|_| layer != ArtworkLayer::Body) {
        let (lw, lh) = entry.label_size_dip;
        let label_x = x + (w - lw) / 2.0;
        let label_y = (y + h + LABEL_GAP_DIP).max(0.0);
        let _ = lh; // reserved for future clipping.
        unsafe {
            ctx.DrawTextLayout(
                Vector2 {
                    X: label_x + 1.0,
                    Y: label_y + 1.0,
                },
                layout,
                brushes.label_shadow,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
            ctx.DrawTextLayout(
                Vector2 { X: label_x, Y: label_y },
                layout,
                brushes.label,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        // Geometric label rect for the snapshot API. Only meaningful
        // when the caller asked for it — the live path passes
        // `None` and pays no DirectWrite roundtrip.
        if let Some(geom) = out_geom {
            geom.icon_rect_dip = Some((icon_left, icon_top, fit_w, fit_h));
            geom.label_rect_dip = label_metrics_rect(layout, label_x, label_y);
            geom.arrow_rect_dip = arrow_rect_dip;
        }
        return;
    }

    if let Some(geom) = out_geom {
        geom.icon_rect_dip = Some((icon_left, icon_top, fit_w, fit_h));
        geom.label_rect_dip = None;
        geom.arrow_rect_dip = arrow_rect_dip;
    }
}

/// Small POD populated by [`draw_icon`] when the snapshot path
/// requests geometry. `None` fields mean "not drawn" — a
/// bitmap-less icon still returns its placeholder rect, but a
/// label-less plan returns `None` for the label rect. `arrow_rect_dip`
/// is populated only when the plan carried a shortcut-arrow bitmap
/// (i.e. the shell reported an arrow overlay and
/// `draw_shortcut_overlay` was on).
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct IconPaintGeometry {
    pub icon_rect_dip: Option<(f32, f32, f32, f32)>,
    pub label_rect_dip: Option<(f32, f32, f32, f32)>,
    pub arrow_rect_dip: Option<(f32, f32, f32, f32)>,
}

/// Compute the actual painted rect of a `IDWriteTextLayout` in
/// overlay-canvas coordinates by adding the layout's
/// `GetMetrics().{left, top, width, height}` to the layout's draw
/// origin. Returns `None` when the layout is empty or the DirectWrite
/// call fails (both are non-fatal — the snapshot pixels are still
/// valid).
fn label_metrics_rect(
    layout: &IDWriteTextLayout,
    origin_x: f32,
    origin_y: f32,
) -> Option<(f32, f32, f32, f32)> {
    let mut m = DWRITE_TEXT_METRICS::default();
    // SAFETY: `GetMetrics` writes exactly one DWRITE_TEXT_METRICS.
    let hr = unsafe { layout.GetMetrics(&mut m) };
    if hr.is_err() {
        return None;
    }
    if m.width <= 0.0 || m.height <= 0.0 {
        return None;
    }
    Some((origin_x + m.left, origin_y + m.top, m.width, m.height))
}

// ---------------------------------------------------------------------------
// Off-screen snapshot renderer
// ---------------------------------------------------------------------------

pub(crate) struct SceneRenderer {
    sprites: Option<crate::sprites::SpriteRenderer>,
    icons: HashMap<IconId, IconEntry>,
    _text: TextResources,
    context: ID2D1DeviceContext,
    placeholder: ID2D1SolidColorBrush,
    label: ID2D1SolidColorBrush,
    shadow: ID2D1SolidColorBrush,
    target: crate::sprites::CaptureTarget,
    canvas: rdi_core::Canvas,
}

impl SceneRenderer {
    pub(crate) fn new(canvas: rdi_core::Canvas, plans: &[IconRenderPlan]) -> Result<Self, DesktopError> {
        canvas.validate()?;
        ensure_dpi_awareness();
        let mut device = None;
        // SAFETY: worker-owned hardware device with valid outputs and BGRA support for D2D.
        unsafe { D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, None) }
            .map_err(crate::sprites::gpu_error)?;
        let device = device.ok_or_else(|| DesktopError::InvalidEffect("missing render device".into()))?;
        let dxgi: IDXGIDevice = device.cast().map_err(crate::sprites::gpu_error)?;
        // SAFETY: D2D context and all artwork share this worker-owned D3D device.
        let context = unsafe { D2D1CreateDevice(&dxgi, None).map_err(crate::sprites::gpu_error)?.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE) }
            .map_err(crate::sprites::gpu_error)?;
        let text = build_text_resources((96.0 * canvas.dpi_scale).round() as u32)?;
        let placeholder = create_brush(&context, D2D1_COLOR_F { r: 0.0, g: 0.7, b: 1.0, a: 0.85 }, "scene placeholder")?;
        let label = create_brush(&context, D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }, "scene label")?;
        let shadow = create_brush(&context, D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.5 }, "scene shadow")?;
        let icons: HashMap<_, _> = plans.iter().map(|plan| (plan.id.clone(), build_icon_entry(&context, &text, plan))).collect();
        let sprites = if plans.iter().any(|plan| plan.effect.is_some()) {
            Some(build_sprite_renderer(&device, &context, &icons, plans, &Brushes { placeholder: &placeholder, label: &label, label_shadow: &shadow })?)
        } else { None };
        let target = crate::sprites::CaptureTarget::new(&device, canvas.width, canvas.height)?;
        Ok(Self { sprites, icons, _text: text, context, placeholder, label, shadow, target, canvas })
    }
}

impl rdi_core::SceneRenderer for SceneRenderer {
    fn render(&mut self, frame: &[rdi_core::IconFrame], seconds: f64) -> Result<rdi_core::CapturedFrame, DesktopError> {
        render_capture(&self.target, self.canvas.width, self.canvas.height, Point::new(0, 0), &self.context,
            &self.icons, &Brushes { placeholder: &self.placeholder, label: &self.label, label_shadow: &self.shadow },
            &mut self.sprites, frame)?;
        self.target.read(seconds)
    }
}

fn render_capture(
    target: &crate::sprites::CaptureTarget, width: u32, height: u32, origin: Point,
    context: &ID2D1DeviceContext, icons: &HashMap<IconId, IconEntry>, brushes: &Brushes<'_>,
    sprites: &mut Option<crate::sprites::SpriteRenderer>, frame: &[rdi_core::IconFrame],
) -> Result<(), DesktopError> {
    if let Some(sprites) = sprites {
        return sprites.render_region(&target.surface, [0.0; 2], RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 }, origin, frame, false);
    }
    let properties = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: 96.0, dpiY: 96.0, bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
        ..Default::default()
    };
    // SAFETY: target and artwork share the worker-owned device; drawing completes and detaches before readback.
    unsafe {
        let bitmap = context.CreateBitmapFromDxgiSurface(&target.surface, Some(&properties)).map_err(crate::sprites::gpu_error)?;
        context.SetTarget(&bitmap);
        context.BeginDraw();
        context.Clear(Some(&D2D1_COLOR_F::default()));
        for entry in frame {
            if let Some(icon) = icons.get(&entry.id) {
                draw_icon(context, brushes, icon, (entry.position.x - origin.x) as f32, (entry.position.y - origin.y) as f32, None);
            }
        }
        let result = context.EndDraw(None, None);
        context.SetTarget(None);
        result.map_err(crate::sprites::gpu_error)
    }
}

/// Render one overlay frame to an off-screen bitmap and return the
/// pixels as tightly-packed premultiplied BGRA (`stride = width * 4`).
///
/// Uses the same D2D device / draw path as
/// [`OverlaySession::render_frame`] but without any `HWND`,
/// DirectComposition target, or display-mode dependency. Safe to call
/// regardless of whether an on-screen overlay session is active.
///
/// `positions` are in **overlay-local pixel coordinates** — i.e. the
/// caller already subtracted the overlay's origin. `dpi_scale` scales
/// the text DPI (via `build_text_resources(96 * dpi_scale)`) so text
/// hinting matches what the live overlay would produce at that scale;
/// icon-bitmap sizing is entirely controlled by the caller-provided
/// `IconRenderPlan::size_px`.
pub(crate) fn render_snapshot(
    width_px: u32,
    height_px: u32,
    dpi_scale: f32,
    plans: &[IconRenderPlan],
    positions: &[(IconId, Point)],
) -> Result<(Vec<u8>, Vec<SnapshotIconGeometry>), DesktopError> {
    ensure_dpi_awareness();

    let width_px = width_px.max(1);
    let height_px = height_px.max(1);
    let dpi_px = ((96.0 * dpi_scale.max(0.01)).round() as u32).max(1);

    // ---- D3D11 → DXGI → D2D chain (no DComp, no HWND) ----------------
    let mut d3d_device_opt: Option<ID3D11Device> = None;
    let hr = unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut d3d_device_opt),
            None,
            None,
        )
    };
    hr.map_err(|e| overlay_err("snapshot D3D11CreateDevice", e))?;
    let d3d_device = d3d_device_opt.ok_or_else(|| {
        DesktopError::OverlayUnavailable(
            "D3D11CreateDevice returned S_OK but no device (snapshot)".into(),
        )
    })?;
    let dxgi_device: IDXGIDevice = d3d_device
        .cast()
        .map_err(|e| overlay_err("snapshot ID3D11Device → IDXGIDevice", e))?;
    let d2d_device: ID2D1Device = unsafe {
        D2D1CreateDevice(&dxgi_device, None)
            .map_err(|e| overlay_err("snapshot D2D1CreateDevice", e))?
    };
    let ctx: ID2D1DeviceContext = unsafe {
        d2d_device
            .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
            .map_err(|e| overlay_err("snapshot CreateDeviceContext", e))?
    };

    // ---- Render-target bitmap ----------------------------------------
    let size = D2D_SIZE_U {
        width: width_px,
        height: height_px,
    };
    let target_props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
        colorContext: std::mem::ManuallyDrop::new(None),
    };
    let target: ID2D1Bitmap1 = unsafe {
        ctx.CreateBitmap(size, None, 0, &target_props)
            .map_err(|e| overlay_err("snapshot CreateBitmap (target)", e))?
    };

    // ---- Staging bitmap for CPU readback -----------------------------
    let staging_props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
        colorContext: std::mem::ManuallyDrop::new(None),
    };
    let staging: ID2D1Bitmap1 = unsafe {
        ctx.CreateBitmap(size, None, 0, &staging_props)
            .map_err(|e| overlay_err("snapshot CreateBitmap (staging)", e))?
    };

    // ---- Text + brushes + icon entries -------------------------------
    let text = build_text_resources(dpi_px)?;
    let placeholder_brush = create_brush(
        &ctx,
        D2D1_COLOR_F {
            r: 0.0,
            g: 0.7,
            b: 1.0,
            a: 0.85,
        },
        "snapshot placeholder_brush",
    )?;
    let label_brush = create_brush(
        &ctx,
        D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        },
        "snapshot label_brush",
    )?;
    let label_shadow_brush = create_brush(
        &ctx,
        D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.5,
        },
        "snapshot label_shadow_brush",
    )?;
    let mut icons: HashMap<IconId, IconEntry> = HashMap::with_capacity(plans.len());
    for plan in plans {
        icons.insert(plan.id.clone(), build_icon_entry(&ctx, &text, plan));
    }

    // ---- Draw --------------------------------------------------------
    let geoms: Vec<SnapshotIconGeometry> = unsafe {
        ctx.SetTarget(&target);
        ctx.BeginDraw();
        ctx.Clear(Some(&D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));
        let brushes = Brushes {
            placeholder: &placeholder_brush,
            label: &label_brush,
            label_shadow: &label_shadow_brush,
        };
        let mut geoms: Vec<SnapshotIconGeometry> = Vec::with_capacity(positions.len());
        for (id, pt) in positions {
            let Some(entry) = icons.get(id) else { continue };
            let mut geom = IconPaintGeometry::default();
            draw_icon(
                &ctx, &brushes, entry,
                pt.x as f32, pt.y as f32,
                Some(&mut geom),
            );
            let Some((ix, iy, iw, ih)) = geom.icon_rect_dip else { continue };
            let icon_rect_px = (
                ix.round() as i32,
                iy.round() as i32,
                iw.round().max(0.0) as u32,
                ih.round().max(0.0) as u32,
            );
            let label_rect_px = geom.label_rect_dip.map(|(lx, ly, lw, lh)| (
                lx.round() as i32,
                ly.round() as i32,
                lw.round().max(0.0) as u32,
                lh.round().max(0.0) as u32,
            ));
            let arrow_rect_px = geom.arrow_rect_dip.map(|(ax, ay, aw, ah)| (
                ax.round() as i32,
                ay.round() as i32,
                aw.round().max(0.0) as u32,
                ah.round().max(0.0) as u32,
            ));
            geoms.push(SnapshotIconGeometry {
                id: id.clone(),
                icon_rect_px,
                label_rect_px,
                arrow_rect_px,
            });
        }
        let mut tag1 = 0u64;
        let mut tag2 = 0u64;
        ctx.EndDraw(Some(&mut tag1), Some(&mut tag2))
            .map_err(|e| overlay_err("snapshot EndDraw", e))?;
        geoms
    };

    // ---- Copy render target → staging, map, repack rows --------------
    unsafe {
        // Cast render-target to ID2D1Bitmap for CopyFromBitmap's src arg.
        let src: ID2D1Bitmap = target
            .cast()
            .map_err(|e| overlay_err("snapshot target.cast::<ID2D1Bitmap>", e))?;
        staging
            .CopyFromBitmap(None, &src, None)
            .map_err(|e| overlay_err("snapshot CopyFromBitmap", e))?;
    }
    let mapped = unsafe {
        staging
            .Map(D2D1_MAP_OPTIONS_READ)
            .map_err(|e| overlay_err("snapshot staging.Map", e))?
    };
    // `mapped.pitch` is bytes per row in the staging surface; may
    // exceed `width_px * 4` due to alignment. Repack to a tight
    // `Vec<u8>` so the returned buffer's length is deterministic and
    // Python-side `Image.frombuffer` doesn't need to know the pitch.
    let row_bytes = (width_px as usize) * 4;
    let src_pitch = mapped.pitch as usize;
    let src_bits = mapped.bits;
    let mut out = Vec::<u8>::with_capacity(row_bytes * height_px as usize);
    for y in 0..height_px as usize {
        // SAFETY: The mapped range is `src_pitch * height_px` bytes
        // per the `ID2D1Bitmap1::Map` contract; indexing stays inside it.
        let row_ptr = unsafe { src_bits.add(y * src_pitch) };
        let row = unsafe { std::slice::from_raw_parts(row_ptr, row_bytes) };
        out.extend_from_slice(row);
    }
    unsafe {
        staging
            .Unmap()
            .map_err(|e| overlay_err("snapshot staging.Unmap", e))?;
    }

    Ok((out, geoms))
}

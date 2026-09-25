//! DirectWrite plumbing for the overlay's icon labels.
//!
//! Owns a single `IDWriteFactory` for the
//! overlay's lifetime plus a per-DPI-cached `IDWriteTextFormat` built
//! from the user's icon-title font (`SPI_GETICONTITLELOGFONT`).
//!
//! # Threading
//!
//! Every call must be on the STA worker thread that owns the
//! `OverlaySession`. DirectWrite objects are thread-safe on paper, but
//! are treated as STA-bound for consistency with the rest of the
//! overlay.

use rdi_core::DesktopError;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_ITALIC, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP, DWriteCreateFactory,
    IDWriteFactory, IDWriteInlineObject, IDWriteTextFormat, IDWriteTextLayout,
};
use windows::Win32::Graphics::Gdi::{LF_FACESIZE, LOGFONTW};
use windows::Win32::UI::HiDpi::SystemParametersInfoForDpi;
use windows::Win32::UI::WindowsAndMessaging::{
    ICONMETRICSW, SPI_GETICONMETRICS, SPI_GETICONTITLEWRAP, SystemParametersInfoW,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};
use windows::core::PCWSTR;

/// Empirical line-height multiplier applied to the font's pixel
/// height. Segoe UI's actual DirectWrite line-box ratio is
/// ~1.36 (`(ascent + descent + lineGap) / emSquare`); 1.5 gives
/// ~10 % headroom so two lines always fit inside
/// `label_h = 2 × line_height`. At 1.2 the box is 76 px, which
/// only fits ONE Segoe UI 30 px line and collapses 2-line titles
/// to a single trimmed line.
const LINE_HEIGHT_FACTOR: f32 = 1.5;

/// Cached DirectWrite state — construct once per `OverlaySession`.
///
/// Field-drop order matters: Rust drops fields top-to-bottom, so the
/// ellipsis `IDWriteInlineObject` (built from `format`) and every
/// `IDWriteTextLayout` created via `build_label_layout` MUST outlive
/// `format` and `factory`. All layouts live inside `OverlaySession`
/// via `IconEntry.label_layout`, and Rust drops the session's
/// per-icon table before the session-owned `TextResources`, so this
/// invariant is upheld today. Do not reorder the fields below
/// without re-checking that DirectWrite is happy releasing them in
/// the new order.
pub(crate) struct TextResources {
    pub(crate) factory: IDWriteFactory,
    pub(crate) format: IDWriteTextFormat,
    /// Font size in DIPs at 96 DPI. Currently unused by the overlay
    /// renderer, but retained here so the value flows out of
    /// `build_text_resources` without a second SPI lookup.
    #[allow(dead_code)]
    pub(crate) size_dip: f32,
    /// Pre-built ellipsis trimming sign attached to every layout.
    /// DirectWrite's `SetTrimming` renders nothing when the inline
    /// object argument is null, so this must be a real sign or long
    /// labels clip without the `…` character.
    pub(crate) trimming_sign: IDWriteInlineObject,
}

/// Build the DirectWrite factory + text format from the user's
/// current icon-title font.
///
/// `dpi_px` is the target physical DPI to size the font for. On
/// Win10 1607+ this drives `SystemParametersInfoForDpi` so the
/// returned `lfFont.lfHeight` matches what Explorer would use at
/// that DPI (12 px @ 96 DPI, 48 px @ 384 DPI on the reference
/// panel). Falls back to the DPI-unaware call on older systems.
///
/// The worker thread's own SPI values cannot be reused: they are
/// resolved for the worker's DPI context, not the target monitor's.
///
/// # Safety
/// Must be called on the same STA thread that will render the labels.
/// The returned interfaces must be dropped on that same thread.
pub(crate) fn build_text_resources(dpi_px: u32) -> Result<TextResources, DesktopError> {
    let factory = create_factory()?;
    let logfont = query_icon_title_logfont_for_dpi(dpi_px);
    let wrap_enabled = query_icon_title_wrap();

    let family = logfont_face_string(&logfont);
    let family_pcwstr = to_pcwstr(&family);

    let weight = if logfont.lfWeight >= 700 {
        DWRITE_FONT_WEIGHT_BOLD
    } else {
        DWRITE_FONT_WEIGHT_NORMAL
    };
    let style = if logfont.lfItalic != 0 {
        DWRITE_FONT_STYLE_ITALIC
    } else {
        DWRITE_FONT_STYLE_NORMAL
    };

    // `lfHeight` from SystemParametersInfoForDpi is already the
    // Explorer pixel height for `dpi_px` (negative = font em height,
    // positive = character cell height; `unsigned_abs` normalises).
    let size_pixels = logfont.lfHeight.unsigned_abs() as f32;
    let size_dip = size_pixels.max(8.0);

    let format: IDWriteTextFormat = unsafe {
        factory
            .CreateTextFormat(
                family_pcwstr,
                None,
                weight,
                style,
                DWRITE_FONT_STRETCH_NORMAL,
                size_dip,
                windows::core::w!("en-US"),
            )
            .map_err(|e| dwrite_err("CreateTextFormat", e))?
    };
    let wrap_mode = if wrap_enabled {
        // Normal word-wrap: break at whitespace, only break mid-word
        // when a single token exceeds the cell width. Matches
        // Explorer's behaviour. `EMERGENCY_BREAK` is explicitly
        // forbidden — it breaks mid-word on every overflow.
        DWRITE_WORD_WRAPPING_WRAP
    } else {
        DWRITE_WORD_WRAPPING_NO_WRAP
    };

    unsafe {
        // Centred horizontally under the icon.
        format
            .SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)
            .map_err(|e| dwrite_err("SetTextAlignment", e))?;
        format
            .SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)
            .map_err(|e| dwrite_err("SetParagraphAlignment", e))?;
        format
            .SetWordWrapping(wrap_mode)
            .map_err(|e| dwrite_err("SetWordWrapping", e))?;
    }

    // Built once here so every layout can share it. A null
    // trimming-sign argument to `SetTrimming` silently disables
    // ellipsis rendering — long labels just clip.
    let trimming_sign: IDWriteInlineObject = unsafe {
        factory
            .CreateEllipsisTrimmingSign(&format)
            .map_err(|e| dwrite_err("CreateEllipsisTrimmingSign", e))?
    };

    Ok(TextResources {
        factory,
        format,
        size_dip,
        trimming_sign,
    })
}

/// Font-derived label metrics used by the backend to size label
/// bounding boxes *before* the overlay session is constructed.
///
/// The overlay itself gets these numbers again (plus DirectWrite
/// resources) from [`build_text_resources`]; this helper is the
/// no-DirectWrite subset the backend needs during `enrich_plans`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LabelMetrics {
    /// Absolute pixel height of the icon-title font at the DPI the
    /// metrics were queried for.
    #[allow(dead_code)]
    pub(crate) font_pixels: f32,
    /// Font pixel height × [`LINE_HEIGHT_FACTOR`]. This is the
    /// nominal per-line vertical advance DirectWrite reserves for
    /// each wrapped line.
    pub(crate) line_height_pixels: f32,
    /// `SPI_GETICONTITLETRAP` — `true` when the user wants desktop
    /// titles to wrap onto multiple lines. Kept for symmetry with
    /// the Win32 struct; the actual wrap decision is delegated to
    /// DirectWrite via [`DWRITE_WORD_WRAPPING_WRAP`].
    #[allow(dead_code)]
    pub(crate) wrap_enabled: bool,
    /// `ICONMETRICSW::iHorzSpacing` — width of the desktop icon
    /// *cell* in physical pixels at the queried DPI. Explorer sizes
    /// label bounding boxes to this value. Falls back to 75 (Windows
    /// default at 100 % scale) if the SPI call fails.
    pub(crate) horz_spacing_pixels: u32,
    /// `ICONMETRICSW::iVertSpacing` — height of the desktop icon
    /// *cell* in physical pixels at the queried DPI. The label
    /// area is the space in this cell below the icon bitmap; used
    /// to derive how many wrap lines Explorer would allow.
    pub(crate) vert_spacing_pixels: u32,
}

/// Query icon-title font + wrap setting at a specific target DPI
/// via `SystemParametersInfoForDpi` (Win10 1607+). Falls back to
/// the DPI-unaware call on older systems.
pub(crate) fn label_metrics_for_dpi(dpi_px: u32) -> LabelMetrics {
    let metrics = query_icon_metrics_for_dpi(dpi_px).or_else(query_icon_metrics);
    labels_from_metrics(metrics)
}

fn labels_from_metrics(metrics: Option<ICONMETRICSW>) -> LabelMetrics {
    let logfont = metrics.map(|m| m.lfFont).unwrap_or_else(fallback_logfont);
    let font_pixels = (logfont.lfHeight.unsigned_abs() as f32).max(8.0);
    let horz_spacing_pixels = metrics
        .and_then(|m| u32::try_from(m.iHorzSpacing).ok())
        .filter(|&w| w > 0)
        .unwrap_or(75);
    let vert_spacing_pixels = metrics
        .and_then(|m| u32::try_from(m.iVertSpacing).ok())
        .filter(|&h| h > 0)
        .unwrap_or(75);
    LabelMetrics {
        font_pixels,
        line_height_pixels: font_pixels * LINE_HEIGHT_FACTOR,
        wrap_enabled: query_icon_title_wrap(),
        horz_spacing_pixels,
        vert_spacing_pixels,
    }
}

/// Build a `IDWriteTextLayout` for a single icon label. The layout
/// is cached per-icon inside `OverlaySession`.
///
/// `max_width_dip` / `max_height_dip` bound the label to Explorer's
/// two-line rule (typically ~2 × `line_height_pixels` for height).
pub(crate) fn build_label_layout(
    resources: &TextResources,
    text: &str,
    max_width_dip: f32,
    max_height_dip: f32,
) -> Result<IDWriteTextLayout, DesktopError> {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let layout: IDWriteTextLayout = unsafe {
        resources
            .factory
            .CreateTextLayout(&wide, &resources.format, max_width_dip, max_height_dip)
            .map_err(|e| dwrite_err("CreateTextLayout", e))?
    };
    // Explorer trims character-by-character (`DT_END_ELLIPSIS`
    // with `DrawText`). WORD granularity is wrong here: a single
    // unbreakable word like a filename "LocalSend-1.4.0" has no
    // interior word boundary, so WORD trimming rolls back to the
    // start and shows essentially just "…". CHARACTER trimming
    // shows as many characters as fit before the ellipsis —
    // matching Explorer's `LocalSend-1…`.
    let trimming = DWRITE_TRIMMING {
        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
        delimiter: 0,
        delimiterCount: 0,
    };
    // `SetTrimming` renders no ellipsis when the inline-object
    // argument is null. Pass the pre-built sign from `TextResources`.
    unsafe {
        layout
            .SetTrimming(&trimming, &resources.trimming_sign)
            .map_err(|e| dwrite_err("SetTrimming", e))?;
    }
    Ok(layout)
}

fn create_factory() -> Result<IDWriteFactory, DesktopError> {
    // SAFETY: `DWriteCreateFactory` returns a fresh IDWriteFactory
    // for the given type. It is never shared across threads.
    let factory: IDWriteFactory = unsafe {
        DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED)
            .map_err(|e| dwrite_err("DWriteCreateFactory", e))?
    };
    Ok(factory)
}

/// Query `SPI_GETICONTITLEWRAP` — the user's "wrap desktop icon
/// titles onto multiple lines" toggle. Defaults to `true` if the
/// call fails, matching the Windows default.
fn query_icon_title_wrap() -> bool {
    // Raw `i32` rather than `BOOL` to sidestep the type shuffle
    // between windows 0.5x / 0.6x releases.
    let mut value: i32 = 1;
    // SAFETY: `SystemParametersInfoW(SPI_GETICONTITLEWRAP)` writes a
    // single `BOOL` (i32) into the buffer. Failure falls back to
    // "wrap on", the Windows default.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETICONTITLEWRAP,
            0,
            Some(&mut value as *mut _ as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if ok.is_ok() { value != 0 } else { true }
}

fn query_icon_title_logfont_for_dpi(dpi_px: u32) -> LOGFONTW {
    query_icon_metrics_for_dpi(dpi_px)
        .or_else(query_icon_metrics)
        .map(|m| m.lfFont)
        .unwrap_or_else(fallback_logfont)
}

/// Fetch `ICONMETRICSW` via `SPI_GETICONMETRICS`. Returns `None` on
/// failure so callers can fall back to their own defaults.
fn query_icon_metrics() -> Option<ICONMETRICSW> {
    // `SPI_GETICONMETRICS` fills an `ICONMETRICSW` struct with the
    // icon-title font (`lfFont`) plus horizontal / vertical cell
    // spacing (`iHorzSpacing` / `iVertSpacing`). Explorer sizes
    // desktop icon cells from the same call.
    let mut metrics: ICONMETRICSW = unsafe { std::mem::zeroed() };
    metrics.cbSize = std::mem::size_of::<ICONMETRICSW>() as u32;
    // SAFETY: `SystemParametersInfoW(SPI_GETICONMETRICS)` writes into
    // the supplied buffer; failure surfaces as `None`.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETICONMETRICS,
            metrics.cbSize,
            Some(&mut metrics as *mut _ as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if ok.is_ok() { Some(metrics) } else { None }
}

/// Same as [`query_icon_metrics`] but pinned to a specific target
/// DPI via `SystemParametersInfoForDpi` (Win10 1607+). Returns
/// `None` when the API is unavailable or the call fails; callers
/// should fall back to [`query_icon_metrics`].
fn query_icon_metrics_for_dpi(dpi_px: u32) -> Option<ICONMETRICSW> {
    let mut metrics: ICONMETRICSW = unsafe { std::mem::zeroed() };
    metrics.cbSize = std::mem::size_of::<ICONMETRICSW>() as u32;
    // SAFETY: forwarded to the writing call below; `metrics` outlives it.
    let ok = unsafe {
        SystemParametersInfoForDpi(
            SPI_GETICONMETRICS.0,
            metrics.cbSize,
            Some(&mut metrics as *mut _ as *mut _),
            0,
            dpi_px,
        )
    };
    if ok.is_ok() { Some(metrics) } else { None }
}

/// Windows 11 default icon-title font (Segoe UI 9 pt). Used when
/// `SPI_GETICONMETRICS` fails outright.
fn fallback_logfont() -> LOGFONTW {
    let mut fallback: LOGFONTW = unsafe { std::mem::zeroed() };
    fallback.lfHeight = -12;
    fallback.lfWeight = 400;
    fallback.lfCharSet.0 = 1; // DEFAULT_CHARSET
    write_face_name(&mut fallback, "Segoe UI");
    fallback
}

fn logfont_face_string(lf: &LOGFONTW) -> Vec<u16> {
    let mut name = lf.lfFaceName.to_vec();
    // Trim at the first NUL — LOGFONTW is a fixed-size UTF-16 buffer.
    if let Some(pos) = name.iter().position(|&c| c == 0) {
        name.truncate(pos);
    }
    if name.is_empty() {
        // Match query_icon_title_logfont's fallback so nothing throws
        // downstream.
        return "Segoe UI".encode_utf16().collect();
    }
    name
}

fn write_face_name(lf: &mut LOGFONTW, name: &str) {
    let wide: Vec<u16> = name.encode_utf16().collect();
    let capacity = LF_FACESIZE as usize;
    let n = wide.len().min(capacity.saturating_sub(1));
    lf.lfFaceName[..n].copy_from_slice(&wide[..n]);
    // Ensure NUL termination.
    if n < capacity {
        lf.lfFaceName[n] = 0;
    }
}

/// Build a `PCWSTR` from a caller-owned UTF-16 vector. The returned
/// pointer is only valid for as long as `bytes` lives — callers must
/// keep the source vector alive across the DirectWrite call.
fn to_pcwstr(bytes: &[u16]) -> PCWSTR {
    // `logfont_face_string` returns strings WITHOUT NUL termination,
    // and DirectWrite needs a NUL-terminated pointer, so the slice is
    // copied into a leaked boxed buffer. One overlay session leaks
    // ~64 bytes, which is acceptable for a per-animation lifetime.
    let mut owned = bytes.to_vec();
    owned.push(0);
    let boxed = owned.into_boxed_slice();
    let ptr = boxed.as_ptr();
    // Intentionally leak — the boxed slice lives until process exit.
    // The overlay-session lifetime is much shorter, so this is a one
    // time cost per unique family name. In practice one leak per
    // process lifetime because the system icon font rarely changes.
    std::mem::forget(boxed);
    PCWSTR(ptr)
}

fn dwrite_err(op: &str, e: windows::core::Error) -> DesktopError {
    DesktopError::BackendUnavailable(format!(
        "DirectWrite {op} failed: HRESULT 0x{:08X} — {}",
        e.code().0 as u32,
        e.message()
    ))
}

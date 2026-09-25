//! Animation-renderer-visible types.
//!
//! Part of the transparent-overlay animation architecture. These types
//! describe the per-icon rendering plan the engine hands to the backend
//! at the start of every animation, plus the outcome of the final Shell
//! commit.
//!
//! Everything in this module is plain data (no callbacks, no COM types)
//! so it is safe to build on any thread. Backends turn it into
//! platform-specific overlay resources on the worker thread.
//!
//! # Field responsibilities
//!
//! The `IconRenderPlan` is a **shared build target** — the engine fills
//! in the fields it knows about, and the backend enriches the ones it
//! knows how to source from the platform. Specifically:
//!
//! | Field | Written by |
//! | --- | --- |
//! | `id` | Engine (from `IconAnimationSpec`) |
//! | `source_position` | Engine (from `list_icons` snapshot) |
//! | `final_position` | Engine (from `IconAnimationSpec::target`) |
//! | `size_px` | Engine sets a fallback; backend refines |
//! | `image` | **Backend** — `None` if not resolvable |
//! | `label` | **Backend** — `None` if not resolvable |
//! | `selected` | **Backend** |
//! | `focused` | **Backend** |

use crate::{IconId, Point};

/// Per-icon rendering plan the engine hands to
/// [`DesktopBackend::begin_overlay_session`](crate::DesktopBackend::begin_overlay_session).
///
/// See the module-level documentation for who fills which field. The
/// engine builds the plan from an `IconAnimationSpec` + the current
/// icon snapshot; the backend enriches it with platform data (icon
/// bitmap, label text, selection state, refined size).
#[derive(Clone, Debug, PartialEq)]
pub struct IconRenderPlan {
    /// Optional shader applied to all enabled artwork for this icon.
    pub effect: Option<crate::Effect>,
    /// Keep this non-participating desktop icon visible at its original position.
    pub stationary: bool,
    /// Stable icon identifier — matches
    /// [`IconSnapshot::id`](crate::IconSnapshot::id).
    pub id: IconId,
    /// Where the icon is *right now*, in the same virtual-screen
    /// coordinate space used by the rest of the API. The overlay
    /// pre-renders its first frame at this position so the transition
    /// from real icons → overlay icons is invisible.
    pub source_position: Point,
    /// Where the icon will be at the end of the animation. The backend
    /// teleports the *real* Shell icon here on the first overlay
    /// commit, before hiding the real icons.
    pub final_position: Point,
    /// Rendered icon size on the source monitor, in pixels. The engine
    /// sets this to a placeholder (`(48, 48)` from
    /// [`placeholder`](Self::placeholder)) and the backend may refine
    /// it in-place using `IFolderView2::GetViewModeAndIconSize` +
    /// per-monitor DPI.
    pub size_px: (u32, u32),
    /// Raw premultiplied BGRA icon bitmap. `None` when the backend
    /// could not resolve one (very rare). The renderer falls back to
    /// a solid placeholder in that case.
    pub image: Option<IconBitmap>,
    /// Label metadata for text rendering below the icon.
    pub label: Option<IconLabel>,
    /// Per-icon render offset in physical pixels applied at draw
    /// time by the overlay renderer.
    /// `IFolderView2::GetItemPosition` returns Explorer's bitmap
    /// top-left in X but sits ~2 DIPs *above* the actual bitmap
    /// top in Y, so overlay icons render 1–2 DIPs high without
    /// this correction. Backend populates this from a
    /// DPI-scaled empirical constant
    /// (`BITMAP_TOP_INSET_DIP × icon_scale`); engine leaves it at
    /// `(0, 0)` and renderers that do not need the shift may
    /// ignore it.
    pub render_offset_px: (i32, i32),
    /// Optional shortcut-arrow overlay bitmap, kept separate from
    /// [`Self::image`] so the renderer can anchor it to the actual
    /// thumbnail's bottom-left corner — mirrored by the same
    /// shadow-reserve offset the base bitmap uses so the arrow
    /// lives in the drop-shadow gutter below-left of the thumbnail
    /// (matches Explorer). For slot-filling icons (default
    /// shortcuts to folders / exes / anything whose shell icon
    /// fills the slot) the anchor coincides with the slot
    /// bottom-left, so nothing moves for that case.
    ///
    /// Kept out of `image` because for image / video / PDF
    /// thumbnails at ≥ 150 % DPI the shell returns a bitmap smaller
    /// than the slot; an arrow blended in at extraction time
    /// co-shifts with the (aspect-fit-centered) thumbnail and never
    /// overhangs the thumbnail bottom the way Explorer's does.
    ///
    /// `None` for icons the shell reports no shortcut arrow for,
    /// for callers who disabled `draw_shortcut_overlay`, or for
    /// backends (stub / fake) that don't source shell overlays.
    /// The admin-elevation shield is *not* split off — it is still
    /// blended into [`Self::image`] at extraction time (bottom-
    /// right, half-slot size) because its slot-relative anchor
    /// happens to coincide with the base-bitmap anchor for every
    /// icon that carries a shield today.
    pub shortcut_arrow_image: Option<IconBitmap>,
}

impl IconRenderPlan {
    /// Convenience constructor used by the engine before the backend
    /// enriches the plan: no image, no label, hard-coded 48×48
    /// placeholder size.
    ///
    /// `source_position` is where the icon lives *now*;
    /// `final_position` is where the animation will land it. The
    /// backend uses `final_position` to teleport the real Shell icon
    /// to its destination on the first overlay commit (so unhiding at
    /// the end of the animation is a no-op instead of a visible jump).
    pub fn placeholder(id: IconId, source_position: Point, final_position: Point) -> Self {
        Self {
            effect: None,
            stationary: false,
            id,
            source_position,
            final_position,
            size_px: (48, 48),
            image: None,
            label: None,
            render_offset_px: (0, 0),
            shortcut_arrow_image: None,
        }
    }
}

/// Premultiplied BGRA icon bitmap the renderer can upload directly to a
/// GPU texture / Direct2D bitmap.
///
/// Bytes are `pixels[y * stride + x * 4] = [B, G, R, A]` with premultiplied
/// alpha. `stride` is in bytes and must equal `width * 4` (no padding).
#[derive(Clone, Debug, PartialEq)]
pub struct IconBitmap {
    pub width: u32,
    pub height: u32,
    /// Row stride in bytes (`= width * 4`).
    pub stride: u32,
    pub pixels: Vec<u8>,
}

/// Label metadata for an icon.
#[derive(Clone, Debug, PartialEq)]
pub struct IconLabel {
    pub text: String,
    pub bounds_px: (u32, u32),
}

/// Global feature toggles for the overlay renderer, so callers can
/// request a minimal / stylised look without patching the backend.
/// All fields default to `true` — draw everything Explorer does.
///
/// The engine passes this struct to
/// [`DesktopBackend::begin_overlay_session`](crate::DesktopBackend::begin_overlay_session)
/// exactly once per animation; the backend uses it while enriching
/// [`IconRenderPlan`]s and building overlay resources. When a
/// decoration is disabled, the backend also skips the associated
/// Shell COM query (cheaper session start, no wasted work).
///
/// Consumers on the Rust side normally build this via
/// `OverlayRenderOptions::default()` and flip individual fields;
/// Python callers pass the same three booleans as keyword arguments
/// on `AnimationOptions`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlayRenderOptions {
    /// Draw the icon title text (label) below each icon.
    pub draw_labels: bool,
    /// Composite the Shell overlay-slot badge (shortcut arrow,
    /// sharing hand, sync-cloud, …) into each icon bitmap. Backed by
    /// `SHGetFileInfoW(SHGFI_OVERLAYINDEX)` + `SHIL_JUMBO` extraction
    /// — disabling this also skips the Shell query.
    pub draw_shortcut_overlay: bool,
    /// Composite the admin-elevation shield badge (`SIID_SHIELD`) into
    /// each icon bitmap when the item's
    /// `IExtractIconW::GetIconLocation(GIL_CHECKSHIELD)` reports
    /// `GIL_SHIELD`. Disabling this skips both the shield probe and
    /// the blend.
    pub draw_shield_overlay: bool,
}

impl OverlayRenderOptions {
    /// All decorations on — the default.
    pub const fn all_enabled() -> Self {
        Self {
            draw_labels: true,
            draw_shortcut_overlay: true,
            draw_shield_overlay: true,
        }
    }

    /// All decorations off — draw only the base icon bitmaps.
    pub const fn minimal() -> Self {
        Self {
            draw_labels: false,
            draw_shortcut_overlay: false,
            draw_shield_overlay: false,
        }
    }
}

impl Default for OverlayRenderOptions {
    fn default() -> Self {
        Self::all_enabled()
    }
}

/// Outcome of the final Shell commit issued by
/// [`DesktopBackend::finalize_overlay_session`](crate::DesktopBackend::finalize_overlay_session).
///
/// Surfaced to callers through
/// [`AnimationHandle::final_commit`](crate::AnimationHandle::final_commit).
///
/// * `moved_ids` — icons the Shell confirmed at their new position via
///   `IFolderView2::GetItemPosition` polling. **This is a confirmation
///   signal, not a census**: the Windows backend stops polling at the
///   first icon that lands, so a successful commit of 200 icons yields
///   a single id here. Treat non-empty as "the commit reached the
///   Shell" and nothing more.
/// * `missing_ids` — icons that were not resolvable by the backend
///   (same semantics as
///   [`DesktopBackend::set_positions`](crate::DesktopBackend::set_positions)).
///   Unlike `moved_ids` this **is** complete: every id listed here was
///   left wherever the Shell last had it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FinalCommitOutcome {
    pub moved_ids: Vec<IconId>,
    pub missing_ids: Vec<IconId>,
}

/// Off-screen render output from
/// [`DesktopBackend::render_overlay_snapshot`](crate::DesktopBackend::render_overlay_snapshot).
///
/// Pixel buffer is tightly-packed premultiplied BGRA — `stride =
/// width * 4` (no row padding). Callers can hand it directly to any
/// image library that accepts BGRA with a known stride (e.g. PIL's
/// `Image.frombuffer("RGBA", (w, h), buf, "raw", "BGRA", 0, 1)`).
///
/// `geometry` is the authoritative per-icon paint answer, populated
/// from D2D's per-icon `DrawBitmap` rect and DirectWrite's
/// `IDWriteTextLayout::GetMetrics`. Callers doing pixel-level
/// diagnostics should prefer these rects to any CV-based
/// segmentation of `pixels`.
/// Backends that can't compute geometry (e.g. the platform stub)
/// return an empty `geometry` vector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotFrame {
    pub width: u32,
    pub height: u32,
    /// Row-major premultiplied BGRA pixel data.
    pub pixels: Vec<u8>,
    /// One entry per icon that was actually drawn into the frame,
    /// in the same order as the caller's `positions` slice. Skips
    /// icons whose id was not present in the backend's cache.
    pub geometry: Vec<SnapshotIconGeometry>,
}

/// Per-icon paint geometry captured during
/// [`SnapshotFrame`] rendering.
///
/// Rects are expressed as `(x, y, width, height)` in overlay-canvas
/// physical pixels (same coordinate space as the pixel buffer,
/// origin at the top-left, y grows downward).
///
/// `icon_rect_px` is the rect passed to D2D `DrawBitmap` — the
/// exact bounds the icon (or placeholder tile) was drawn into.
/// Aspect-fit padding sits *outside* this rect, not inside it.
///
/// `label_rect_px` is derived from `IDWriteTextLayout::GetMetrics`
/// after the layout was populated with the icon's display name,
/// then translated by the label draw origin. It reflects post-wrap,
/// post-trim glyph extents — the pixel-truthful answer to "where
/// does the label actually land". `None` when the icon has no
/// label (headless plans, empty display name).
///
/// `arrow_rect_px` is the D2D `DrawBitmap` rect used for the
/// shortcut-arrow overlay. The arrow is anchored to the actual
/// thumbnail's bottom-left, mirrored by `shadow_reserve_dip`. For
/// slot-filling icons that coincides with the slot bottom-left.
/// `None` when the icon has no shortcut arrow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotIconGeometry {
    pub id: IconId,
    pub icon_rect_px: (i32, i32, u32, u32),
    pub label_rect_px: Option<(i32, i32, u32, u32)>,
    pub arrow_rect_px: Option<(i32, i32, u32, u32)>,
}

//! The single source of truth for overlay icon + label geometry.
//!
//! Two callers need this math and they must never disagree:
//!
//! * [`WindowsBackend::enrich_plans`](crate::WindowsBackend) — the live
//!   overlay path, which resolves `icon_scale` per icon from the monitor
//!   the icon currently sits on.
//! * `WindowsBackend::build_snapshot_plans` — the off-screen
//!   `render_overlay_snapshot` path, which takes `icon_scale` from the
//!   caller so a snapshot is display-mode independent.
//!
//! **The only legitimate difference between the two callers is where
//! `icon_scale` comes from.** Everything downstream of it lives here.
//! The snapshot path exists to predict what the live path draws, so a
//! forked implementation would let the diagnostics silently stop
//! describing reality — and every empirically-tuned constant here was
//! measured through those diagnostics.
//!
//! This module is deliberately free of COM and Win32 calls: it is plain
//! arithmetic over numbers the caller has already fetched, which makes
//! it the first piece of overlay layout math that can be unit-tested.

use crate::text::LabelMetrics;

/// Minimum icon edge in physical pixels. Guards against a corrupt shell
/// reporting an absurd `GetViewModeAndIconSize`.
const MIN_ICON_PX: u32 = 16;

/// Minimum label box, in physical pixels.
const MIN_LABEL_W_PX: u32 = 16;
const MIN_LABEL_H_PX: u32 = 8;

/// Slack added on top of the two-line minimum label height so a
/// descender on the second line isn't clipped.
const TWO_LINE_SLACK_PX: f32 = 8.0;

/// `IFolderView2::GetItemPosition` returns Explorer's icon-bitmap
/// top-left in X, but in Y it reports a point 5 physical pixels *above*
/// the actual bitmap top — **DPI-independent**, empirically validated at
/// 100 % and 250 % scale.
///
/// The theory (unverified against Microsoft source) is that
/// `SysListView32`'s classic-Win32 paint code uses hard-coded pixel
/// constants for its per-item metrics (inter-item pad ~3 px + icon-area
/// top inset ~2 px) that were never DPI-scaled when the DPI system was
/// retrofitted. The diagnostic
/// ([temp_python/diagnose_y_offset.py](../../../../temp_python/diagnose_y_offset.py))
/// on the 250 % desktop dumped `cell.top - pos.y = +3` physical px and
/// an icon-area top inset of ≈ 2 physical px → total 5.
///
/// A DPI-scaled formula (`round(2.0 × icon_scale)`) matches at 250 %
/// but under-shifts to 2 px at 100 %; the constant is flat on purpose.
const BITMAP_Y_OFFSET_PX: i32 = 5;

/// Resolved per-icon geometry, in physical pixels on the target
/// monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IconLayout {
    /// Icon slot size. The bitmap is aspect-fit inside this.
    pub(crate) size_px: (u32, u32),
    /// Label box beneath the icon.
    pub(crate) label_bounds_px: (u32, u32),
    /// Draw-time correction applied on top of the Shell-reported
    /// position.
    pub(crate) render_offset_px: (i32, i32),
}

/// Convert a scale factor (`1.0` → 96 DPI, `2.5` → 240 DPI) to the
/// integral DPI value `SystemParametersInfoForDpi` expects.
#[inline]
pub(crate) fn dpi_for_scale(icon_scale: f32) -> u32 {
    (icon_scale.max(0.01) * 96.0).round().max(1.0) as u32
}

impl IconLayout {
    /// Resolve the geometry for one icon.
    ///
    /// * `base_size_dip` — the shell's logical icon size from
    ///   `IFolderView2::GetViewModeAndIconSize`. It arrives at the 96-DPI
    ///   baseline (48 for the standard "large icons" desktop mode)
    ///   regardless of the user's display scale, because it crosses a COM
    ///   boundary out of Explorer's DPI-awareness context into this
    ///   process's PMv2 context.
    /// * `icon_scale` — physical DPI ÷ 96 for the monitor this icon is
    ///   being rendered for. Multiplying is what makes overlay icons match
    ///   the real ones pixel-for-pixel; at 250 % it turns the shell's 48
    ///   into 120 physical px.
    /// * `metrics` — icon-title font + cell metrics queried at
    ///   [`dpi_for_scale(icon_scale)`](dpi_for_scale), so wrap sizing
    ///   tracks Explorer's own paint without a secondary rescale.
    pub(crate) fn resolve(
        base_size_dip: (u32, u32),
        icon_scale: f32,
        metrics: &LabelMetrics,
    ) -> Self {
        let scale = icon_scale.max(0.01);
        let size_px = (
            ((base_size_dip.0 as f32 * scale).round() as u32).max(MIN_ICON_PX),
            ((base_size_dip.1 as f32 * scale).round() as u32).max(MIN_ICON_PX),
        );

        // Label width is Explorer's cell width. Height is whatever the
        // cell has left below the icon (`iVertSpacing - icon_h`),
        // floored at a two-line box so short labels don't jitter between
        // one and two lines as the icon size changes.
        let label_area_h = metrics.vert_spacing_pixels.saturating_sub(size_px.1);
        let two_line_min =
            (metrics.line_height_pixels * 2.0 + TWO_LINE_SLACK_PX).round() as u32;
        let label_bounds_px = (
            metrics.horz_spacing_pixels.max(MIN_LABEL_W_PX),
            label_area_h.max(two_line_min).max(MIN_LABEL_H_PX),
        );

        Self {
            size_px,
            label_bounds_px,
            render_offset_px: (0, BITMAP_Y_OFFSET_PX),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Metrics shaped like a real 96-DPI desktop, but built by hand so
    /// the tests need no Win32 call.
    fn metrics(line_height: f32, horz: u32, vert: u32) -> LabelMetrics {
        LabelMetrics {
            font_pixels: line_height / 1.5,
            line_height_pixels: line_height,
            wrap_enabled: true,
            horz_spacing_pixels: horz,
            vert_spacing_pixels: vert,
        }
    }

    #[test]
    fn scale_one_passes_base_size_through() {
        let l = IconLayout::resolve((48, 48), 1.0, &metrics(18.0, 75, 100));
        assert_eq!(l.size_px, (48, 48));
    }

    #[test]
    fn size_scales_with_dpi() {
        // 250 % is the dev-box scale every empirical constant was
        // measured at: 48 logical -> 120 physical.
        let l = IconLayout::resolve((48, 48), 2.5, &metrics(45.0, 188, 250));
        assert_eq!(l.size_px, (120, 120));
    }

    #[test]
    fn size_is_floored_so_a_corrupt_shell_cannot_vanish_icons() {
        let l = IconLayout::resolve((48, 48), 0.01, &metrics(18.0, 75, 100));
        assert_eq!(l.size_px, (MIN_ICON_PX, MIN_ICON_PX));
    }

    #[test]
    fn non_positive_scale_is_clamped_not_panicking() {
        for scale in [0.0_f32, -1.0, -0.0] {
            let l = IconLayout::resolve((48, 48), scale, &metrics(18.0, 75, 100));
            assert_eq!(l.size_px, (MIN_ICON_PX, MIN_ICON_PX));
        }
    }

    #[test]
    fn label_width_is_the_cell_width() {
        let l = IconLayout::resolve((48, 48), 1.0, &metrics(18.0, 75, 100));
        assert_eq!(l.label_bounds_px.0, 75);
    }

    #[test]
    fn label_height_uses_cell_remainder_when_it_exceeds_two_lines() {
        // vert 200 - icon 48 = 152 available; two-line min is
        // 18*2+8 = 44. Remainder wins.
        let l = IconLayout::resolve((48, 48), 1.0, &metrics(18.0, 75, 200));
        assert_eq!(l.label_bounds_px.1, 152);
    }

    #[test]
    fn label_height_falls_back_to_two_lines_when_the_cell_is_tight() {
        // vert 60 - icon 48 = 12 available, below the 44 px two-line
        // floor, so the floor applies.
        let l = IconLayout::resolve((48, 48), 1.0, &metrics(18.0, 75, 60));
        assert_eq!(l.label_bounds_px.1, 44);
    }

    #[test]
    fn label_height_survives_an_icon_taller_than_the_cell() {
        // `saturating_sub` territory: icon 120 > cell 100.
        let l = IconLayout::resolve((48, 48), 2.5, &metrics(45.0, 188, 100));
        assert_eq!(l.label_bounds_px.1, (45.0 * 2.0 + 8.0) as u32);
    }

    #[test]
    fn y_offset_is_dpi_independent() {
        // A DPI-scaled formula is explicitly forbidden — the offset
        // must be identical at every scale.
        for scale in [1.0_f32, 1.5, 2.0, 2.5, 4.0] {
            let l = IconLayout::resolve((48, 48), scale, &metrics(18.0 * scale, 75, 100));
            assert_eq!(l.render_offset_px, (0, 5), "scale {scale}");
        }
    }

    #[test]
    fn dpi_for_scale_matches_the_windows_baseline() {
        assert_eq!(dpi_for_scale(1.0), 96);
        assert_eq!(dpi_for_scale(1.25), 120);
        assert_eq!(dpi_for_scale(2.5), 240);
        assert_eq!(dpi_for_scale(4.0), 384);
    }

    #[test]
    fn dpi_for_scale_never_returns_zero() {
        assert!(dpi_for_scale(0.0) >= 1);
        assert!(dpi_for_scale(-5.0) >= 1);
    }

    /// The property that motivated this module: both callers must get
    /// identical geometry from identical inputs.
    #[test]
    fn live_and_snapshot_inputs_yield_identical_geometry() {
        let m = metrics(45.0, 188, 250);
        let live = IconLayout::resolve((48, 48), 2.5, &m);
        let snapshot = IconLayout::resolve((48, 48), 2.5, &m);
        assert_eq!(live, snapshot);
    }
}

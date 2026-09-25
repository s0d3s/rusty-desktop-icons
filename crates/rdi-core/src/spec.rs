//! Animation specification types.
//!
//! The engine consumes a `Vec<IconAnimationSpec>` and an
//! [`AnimationOptions`]; both are plain data with no callbacks.

use crate::curve::Curve;
use crate::duration::Duration;
use crate::geometry::Point;
use crate::id::IconId;
use crate::overlay_plan::OverlayRenderOptions;

/// Reusable movement, effect-strength and duration recommendations.
///
/// Pass these fields to animation/effect constructors explicitly. A preset
/// does not compile shaders, change their progress clock or start playback.
/// The default is ease-in-out movement, smooth 15 percent strength fades and
/// a fixed two-second duration. This value is available on every platform.
#[derive(Clone, Debug)]
pub struct AnimationPreset {
    /// Suggested position interpolation, reusable for either axis.
    pub movement: Curve,
    /// Suggested effect strength; Effect still enforces clean endpoints.
    pub envelope: Curve,
    /// Suggested fixed or distance-based timing policy.
    pub duration: Duration,
}

impl Default for AnimationPreset {
    fn default() -> Self {
        Self {
            movement: Curve::ease_in_out(),
            envelope: Curve::keyframes(
                vec![
                    crate::Keyframe::new(0.0, 0.0),
                    crate::Keyframe::new(0.15, 1.0),
                    crate::Keyframe::new(0.85, 1.0),
                    crate::Keyframe::new(1.0, 0.0),
                ],
                crate::KeyframeInterp::SmoothStep,
            ).expect("valid default preset envelope"),
            duration: Duration::fixed(std::time::Duration::from_secs(2)),
        }
    }
}

/// Per-icon animation description.
///
/// X and Y are two independent axis animations that share the same
/// normalised time `t = elapsed / duration ∈ [0, 1]`, so both axes tick in
/// lock-step but can use different curves.
#[derive(Clone, Debug)]
pub struct IconAnimationSpec {
    pub id: IconId,
    pub target: Point,
    pub duration: Duration,
    pub curve_x: Curve,
    pub curve_y: Curve,
    pub effect: Option<crate::Effect>,
}

impl IconAnimationSpec {
    /// Convenience constructor that reuses the same curve for both axes.
    pub fn new(id: IconId, target: Point, duration: Duration, curve: Curve) -> Self {
        Self {
            id,
            target,
            duration,
            curve_x: curve.clone(),
            curve_y: curve,
            effect: None,
        }
    }

    /// Constructor with distinct X/Y curves.
    pub fn with_axes(
        id: IconId,
        target: Point,
        duration: Duration,
        curve_x: Curve,
        curve_y: Curve,
    ) -> Self {
        Self {
            id,
            target,
            duration,
            curve_x,
            curve_y,
            effect: None,
        }
    }
}

/// Global options that apply to every icon in an animation batch.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnimationOptions {
    /// Align destinations to available grid cells before preparation. Protects
    /// stationary icons and resolves target conflicts; never changes Shell flags.
    /// Defaults to false. Failures occur before artwork, flags or position writes.
    pub snap_to_grid: bool,

    /// Tick rate in Hz. Defaults to 100 (matching the legacy 10 ms step).
    pub tick_hz: Option<u32>,

    /// Distance threshold in pixels below which an icon is considered "at
    /// target" and dropped from the active set. Defaults to `10`
    /// (matching the legacy `POSITION_ERROR_VALUE`).
    pub position_tolerance_px: Option<i32>,

    /// Optional folder-flag operation applied before the animation starts.
    pub before_flags: Option<FolderFlagOp>,

    /// Optional folder-flag operation applied after the animation ends
    /// (regardless of success or user-triggered stop).
    pub after_flags: Option<FolderFlagOp>,

    /// Force the engine to skip the overlay entirely and take the
    /// "loud warning + direct teleport" fallback path.
    ///
    /// Lets tests and the `animate_direct` demo binary exercise the
    /// fallback without needing a real overlay failure. Defaults to
    /// `false` — normal callers get the overlay animation.
    pub force_fallback: bool,

    /// Feature toggles for the overlay renderer's decoration passes
    /// (label text, shortcut-arrow overlay, admin shield overlay).
    /// Defaults to [`OverlayRenderOptions::all_enabled`] — every
    /// decoration Explorer draws is drawn on the overlay too.
    pub render_options: OverlayRenderOptions,
}

/// Folder-flag mutation semantics, mirroring the legacy `ISF_*` modifiers.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FolderFlagOp {
    /// OR-set semantics — matches legacy `set_desktop_flags`.
    ///
    /// The engine will call `SetCurrentFolderFlags(flags, 0xFFFF_FFFF)`.
    Set(u32),

    /// Exactly-set semantics — matches legacy `exactly_set_desktop_flags`.
    ///
    /// The engine will call `SetCurrentFolderFlags(build_true_mask(flags), flags)`.
    Exactly(u32),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Duration, IconId};

    #[test]
    fn animation_preset_defaults_are_independent() {
        use crate::AnimationCurve;
        let mut preset = AnimationPreset::default();
        assert_eq!(preset.movement, Curve::ease_in_out());
        assert_eq!(preset.duration.resolve(Point::ZERO, Point::ZERO).unwrap(), std::time::Duration::from_secs(2));
        for (progress, expected) in [(0.0, 0.0), (0.15, 1.0), (0.5, 1.0), (0.85, 1.0), (1.0, 0.0)] {
            assert_eq!(preset.envelope.eval(progress), expected);
        }
        preset.movement = Curve::linear();
        assert_eq!(AnimationPreset::default().movement, Curve::ease_in_out());
        fn assert_send_sync<Value: Send + Sync>() {}
        assert_send_sync::<AnimationPreset>();
    }

    #[test]
    fn spec_new_shares_curve_between_axes() {
        let c = Curve::linear();
        let s = IconAnimationSpec::new(
            IconId::from("x"),
            Point::new(10, 20),
            Duration::fixed(std::time::Duration::from_millis(500)),
            c.clone(),
        );
        assert_eq!(s.curve_x, c);
        assert_eq!(s.curve_y, c);
    }

    #[test]
    fn options_default_leaves_everything_none() {
        let o = AnimationOptions::default();
        assert!(o.tick_hz.is_none());
        assert!(o.position_tolerance_px.is_none());
        assert!(o.before_flags.is_none());
        assert!(o.after_flags.is_none());
        assert!(!o.force_fallback);
        assert_eq!(o.render_options, OverlayRenderOptions::all_enabled());
    }
}

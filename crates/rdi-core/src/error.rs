//! Error types for the `rdi-core` crate.

use thiserror::Error;

use crate::IconId;

/// Errors returned by desktop-facing operations.
///
/// `#[non_exhaustive]` so variants can be added without a breaking
/// change.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum DesktopError {
    /// The underlying backend could not be initialised or is temporarily
    /// unavailable (e.g. the desktop `IFolderView2` could not be acquired).
    #[error("desktop backend is unavailable: {0}")]
    BackendUnavailable(String),

    /// A COM / Win32 call returned a failure `HRESULT`.
    #[error("COM error (0x{hresult:08X}): {msg}")]
    Com { hresult: u32, msg: String },

    /// A requested icon was not present in the current enumeration.
    #[error("icon not found: {0}")]
    IconNotFound(IconId),

    /// A curve failed validation.
    #[error("invalid curve: {0}")]
    InvalidCurve(#[from] CurveError),

    /// A duration policy could not be resolved to a concrete `Duration`.
    #[error("invalid duration: {0}")]
    InvalidDuration(String),

    #[error("cannot resolve desktop grid: {0}")]
    InvalidGrid(String),

    /// Shader source, constants or renderer contract is invalid.
    #[error("invalid effect: {0}")]
    InvalidEffect(String),

    /// The current OS has no backend implementation.
    #[error("this platform is not supported by any compiled backend")]
    UnsupportedPlatform,

    /// The animation worker thread has stopped unexpectedly.
    #[error("animation worker crashed: {0}")]
    WorkerCrashed(String),

    /// A concurrent animation is already in progress and the caller asked
    /// for exclusive access.
    #[error("another animation is already running")]
    AnimationBusy,

    /// The backend cannot open a transparent overlay for animation
    /// rendering — the platform doesn't support it, or a compatibility
    /// probe failed. The engine falls back to a direct
    /// [`DesktopBackend::set_positions`](crate::DesktopBackend::set_positions)
    /// teleport when it sees this variant (with a loud warning).
    #[error("overlay renderer unavailable: {0}")]
    OverlayUnavailable(String),

    /// The overlay renderer aborted the current session because the
    /// environment underneath it changed in a way that invalidates
    /// its cached state — typically Explorer restarting, the display
    /// topology changing, or the shell view HWND vanishing.
    ///
    /// The engine treats this as a **graceful stop** (equivalent to
    /// [`FinishReason::Stopped(StopMode::TeleportToTarget)`](crate::events::FinishReason::Stopped)):
    /// the real icons were teleported to their final positions on the
    /// first overlay commit, so the animation ends at a correct steady
    /// state. Finalisation runs as usual to unhide the real icons and
    /// destroy the overlay window.
    #[error("overlay animation cancelled: {0}")]
    OverlayCancelled(String),
}

/// Errors raised while constructing or sampling curves.
#[non_exhaustive]
#[derive(Debug, Error, PartialEq)]
pub enum CurveError {
    #[error("keyframe curve must contain at least 2 keys, got {0}")]
    TooFewKeys(usize),

    #[error(
        "keyframe times must be strictly ascending in [0, 1] \
         (bad key at index {index}: t = {t})"
    )]
    InvalidKeyOrder { index: usize, t: f32 },

    #[error("first keyframe must have t = 0.0, got t = {0}")]
    FirstKeyNotZero(f32),

    #[error("last keyframe must have t = 1.0, got t = {0}")]
    LastKeyNotOne(f32),

    #[error("keyframe value is not finite (index {index}, t = {t}, v = {v})")]
    NonFiniteValue { index: usize, t: f32, v: f32 },

    #[error(
        "sampled function returned an out-of-range or non-finite value at t = {t}: v = {v}"
    )]
    SampledValueInvalid { t: f32, v: f32 },

    #[error("sample count out of bounds: got {got}, expected 2..=4096")]
    InvalidSampleCount { got: u32 },

    #[error(
        "invalid cubic-Bézier control points: c1x = {c1x}, c2x = {c2x} \
         (both must lie in [0, 1])"
    )]
    InvalidBezier { c1x: f32, c2x: f32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_error_display_covers_variants() {
        assert!(format!("{}", DesktopError::UnsupportedPlatform).contains("platform"));
        assert!(format!("{}", DesktopError::AnimationBusy).contains("already running"));
        let e = DesktopError::Com {
            hresult: 0x8000_4005,
            msg: "boom".into(),
        };
        assert!(format!("{e}").contains("0x80004005"));
    }

    #[test]
    fn curve_error_into_desktop_error_via_from() {
        let e: DesktopError = CurveError::TooFewKeys(1).into();
        assert!(matches!(e, DesktopError::InvalidCurve(_)));
    }
}

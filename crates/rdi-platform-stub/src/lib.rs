//! Non-Windows placeholder backend.
//!
//! Every operation returns [`rdi_core::DesktopError::UnsupportedPlatform`]
//! so the workspace continues to compile and test on Linux / macOS.
//!
//! No `unsafe` is used or permitted in this crate.

use rdi_core::{
    DesktopBackend, DesktopError, FinalCommitOutcome, IconId, IconRenderPlan, IconSnapshot,
    MonitorInfo, OverlayRenderOptions, Point,
};

/// Stub backend that always reports `UnsupportedPlatform`.
#[derive(Debug, Default, Clone, Copy)]
pub struct StubBackend;

impl StubBackend {
    #[inline]
    pub const fn new() -> Self {
        Self
    }

    /// Returns [`DesktopError::UnsupportedPlatform`] verbatim.
    #[inline]
    pub fn unsupported<T>() -> Result<T, DesktopError> {
        Err(DesktopError::UnsupportedPlatform)
    }
}

impl DesktopBackend for StubBackend {
    fn list_icons(&mut self) -> Result<Vec<IconSnapshot>, DesktopError> {
        Self::unsupported()
    }

    fn get_flags(&mut self) -> Result<u32, DesktopError> {
        Self::unsupported()
    }

    fn apply_flags(&mut self, _mask: u32, _values: u32) -> Result<(), DesktopError> {
        Self::unsupported()
    }

    fn set_positions(
        &mut self,
        _moves: &[(IconId, Point)],
    ) -> Result<Vec<IconId>, DesktopError> {
        Self::unsupported()
    }

    fn list_monitors(&mut self) -> Result<Vec<MonitorInfo>, DesktopError> {
        Self::unsupported()
    }

    fn begin_overlay_session(
        &mut self,
        _plans: &[IconRenderPlan],
        _render_options: OverlayRenderOptions,
    ) -> Result<(), DesktopError> {
        // Every stub-platform call surfaces the same "no backend
        // compiled for this OS" error via the more-specific
        // `OverlayUnavailable` variant so the engine's fallback path
        // has a well-typed way to distinguish "no overlay" from
        // "backend genuinely dead".
        Err(DesktopError::OverlayUnavailable(
            "no desktop backend compiled for this platform".to_string(),
        ))
    }

    fn commit_overlay_frame(
        &mut self,
        _positions: &[(IconId, Point)],
    ) -> Result<(), DesktopError> {
        Self::unsupported()
    }

    fn finalize_overlay_session(
        &mut self,
        _final_positions: &[(IconId, Point)],
    ) -> Result<FinalCommitOutcome, DesktopError> {
        Self::unsupported()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_reports_unsupported() {
        let err = StubBackend::unsupported::<()>().unwrap_err();
        assert!(matches!(err, DesktopError::UnsupportedPlatform));
    }

    #[test]
    fn stub_backend_all_ops_unsupported() {
        let mut b = StubBackend::new();
        assert!(matches!(
            b.list_icons().unwrap_err(),
            DesktopError::UnsupportedPlatform
        ));
        assert!(matches!(
            b.get_flags().unwrap_err(),
            DesktopError::UnsupportedPlatform
        ));
        assert!(matches!(
            b.apply_flags(0, 0).unwrap_err(),
            DesktopError::UnsupportedPlatform
        ));
        assert!(matches!(
            b.set_positions(&[]).unwrap_err(),
            DesktopError::UnsupportedPlatform
        ));
        assert!(matches!(
            b.list_monitors().unwrap_err(),
            DesktopError::UnsupportedPlatform
        ));
        assert!(matches!(b.desktop_info(&[]), Err(DesktopError::UnsupportedPlatform)));
    }
}

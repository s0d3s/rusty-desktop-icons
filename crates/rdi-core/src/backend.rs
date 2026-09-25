//! The backend abstraction that the animation engine drives.
//!
//! Every backend runs on the engine's dedicated worker thread and is
//! therefore free to hold non-`Sync` platform state (COM interfaces, PIDL
//! caches, GPU devices, etc.). The trait only requires `Send + 'static`
//! so the worker thread can own it exclusively.

use crate::{
    DesktopError, FinalCommitOutcome, IconId, IconRenderPlan, IconSnapshot, MonitorInfo,
    OverlayRenderOptions, Point, SnapshotFrame,
};

/// The single point of contact between the animation engine and the OS
/// desktop.
///
/// # Threading
///
/// A `DesktopBackend` is **owned** by the engine's worker thread and is
/// never accessed concurrently. It only needs to be `Send`, not `Sync`.
///
/// # Method contracts
///
/// * [`list_icons`] returns a snapshot of every icon currently visible
///   on the desktop. Order is unspecified.
/// * [`get_flags`] returns the desktop folder-view flags word verbatim.
/// * [`apply_flags(mask, values)`] performs a masked update:
///   `new_flags = (old_flags & !mask) | (values & mask)` — matching the
///   `IFolderView2::SetCurrentFolderFlags` semantics used by the legacy
///   Windows implementation.
/// * [`set_positions`](DesktopBackend::set_positions) commits the entire batch atomically (from
///   the caller's perspective) and returns the subset of ids that could
///   not be resolved by the backend so the engine can drop them from the
///   active set. Used by the direct-positioning API **and** by the
///   engine's overlay-unavailable fallback path.
/// * [`list_monitors`] returns every connected display in the virtual-
///   screen coordinate system icon positions use. Order is unspecified
///   but the primary monitor always has `is_primary = true`.
/// * [`begin_overlay_session`] / [`commit_overlay_frame`] /
///   [`finalize_overlay_session`] together drive the overlay-based
///   animation flow — see the trio's individual docs.
///
/// [`list_icons`]: DesktopBackend::list_icons
/// [`get_flags`]: DesktopBackend::get_flags
/// [`apply_flags`]: DesktopBackend::apply_flags
/// [`set_positions`]: DesktopBackend::set_positions
/// [`list_monitors`]: DesktopBackend::list_monitors
/// [`begin_overlay_session`]: DesktopBackend::begin_overlay_session
/// [`commit_overlay_frame`]: DesktopBackend::commit_overlay_frame
/// [`finalize_overlay_session`]: DesktopBackend::finalize_overlay_session
pub trait DesktopBackend: Send + 'static {
    fn list_icons(&mut self) -> Result<Vec<IconSnapshot>, DesktopError>;

    fn get_flags(&mut self) -> Result<u32, DesktopError>;

    fn apply_flags(&mut self, mask: u32, values: u32) -> Result<(), DesktopError>;

    /// Commit a batch of `(id, new_position)` moves. Returns the ids the
    /// backend could not resolve (i.e. icons that vanished or were never
    /// on the desktop).
    fn set_positions(
        &mut self,
        moves: &[(IconId, Point)],
    ) -> Result<Vec<IconId>, DesktopError>;

    /// Enumerate every connected display. Bounds are reported in the
    /// same virtual-screen coordinate system used by
    /// [`list_icons`](Self::list_icons) and
    /// [`set_positions`](Self::set_positions).
    fn list_monitors(&mut self) -> Result<Vec<MonitorInfo>, DesktopError>;

    /// Query live display/grid metrics using the worker's current icon snapshot.
    /// Must not reposition icons or change folder flags.
    fn desktop_info(&mut self, _icons: &[IconSnapshot]) -> Result<crate::DesktopInfo, DesktopError> {
        Err(DesktopError::UnsupportedPlatform)
    }

    // -----------------------------------------------------------------
    // Overlay-based animation.
    // -----------------------------------------------------------------

    /// Prepare an overlay animation session covering the icons in `plans`.
    ///
    /// The backend acquires whatever OS resources it needs (window,
    /// renderer, icon bitmaps), pre-renders the first frame at each
    /// icon's *source* position, and returns success only after the
    /// overlay is ready to display that first frame.
    ///
    /// On success the backend has **not yet**:
    ///   * shown the overlay window,
    ///   * hidden the real desktop icons.
    ///
    /// Both happen implicitly on the first call to
    /// [`commit_overlay_frame`](Self::commit_overlay_frame).
    ///
    /// # Errors
    /// * [`DesktopError::OverlayUnavailable`] — the platform doesn't
    ///   support overlay rendering, or a documented compatibility probe
    ///   failed. The engine handles this by taking the loud-warning
    ///   fallback path.
    /// * any other `DesktopError` — engine surfaces as
    ///   [`FinishReason::Error`](crate::events::FinishReason::Error)
    ///   and aborts the animation without moving the icons.
    fn begin_overlay_session(
        &mut self,
        plans: &[IconRenderPlan],
        render_options: OverlayRenderOptions,
    ) -> Result<(), DesktopError>;

    /// Update the overlay's rendered icon positions to `positions`.
    ///
    /// The first call to this method also, atomically from the caller's
    /// perspective:
    ///   * shows the overlay window,
    ///   * waits for at least one composition frame to reach the display,
    ///   * hides the real desktop icons via the Shell's own visibility
    ///     mechanism.
    ///
    /// Subsequent calls only update the overlay's icon-copy positions
    /// and re-present. **No** contact is made with `IFolderView2`.
    fn commit_overlay_frame(
        &mut self,
        positions: &[(IconId, Point)],
    ) -> Result<(), DesktopError>;

    /// Commit positions and visual clocks together. Non-GPU backends ignore visuals.
    fn commit_visual_frame(&mut self, frame: &[crate::IconFrame]) -> Result<(), DesktopError> {
        let positions: Vec<_> = frame.iter().map(|entry| (entry.id.clone(), entry.position)).collect();
        self.commit_overlay_frame(&positions)
    }

    /// Discard a prepared, never-started session without touching Shell positions.
    fn discard_overlay_session(&mut self) {}

    /// Reject a prepared snapshot invalidated by environment changes, before any Shell writes.
    fn validate_prepared_session(&mut self) -> Result<(), DesktopError> { Ok(()) }

    fn prepare_scene_renderer(&mut self, _canvas: crate::Canvas, _plans: &[IconRenderPlan], _options: OverlayRenderOptions) -> Result<Box<dyn crate::SceneRenderer>, DesktopError> {
        Err(DesktopError::UnsupportedPlatform)
    }

    fn capture_overlay(&mut self, _seconds: f64) -> Result<crate::CapturedFrame, DesktopError> {
        Err(DesktopError::UnsupportedPlatform)
    }

    fn set_real_icons_visible(&mut self, _visible: bool) -> Result<(), DesktopError> {
        Err(DesktopError::UnsupportedPlatform)
    }

    fn poll_overlay_session(&mut self) -> Result<(), DesktopError> {
        self.validate_prepared_session()
    }

    /// End the animation session and commit the final positions to the
    /// Shell.
    ///
    /// Sequence:
    ///   1. `IFolderView2::SelectAndPositionItems(final_positions,
    ///      SVSI_POSITIONITEM)` — one call, all icons, matching legacy.
    ///   2. Poll `IFolderView2::GetItemPosition` until at least one
    ///      moved icon reports its new position (up to a short timeout).
    ///   3. Restore real-icon visibility.
    ///   4. Force one desktop repaint.
    ///   5. Hide + destroy the overlay window.
    ///
    /// This method **must** clear any Shell-side icon-hiding flag it
    /// applied, even on failure paths — the engine relies on this
    /// invariant to guarantee the real icons come back after an
    /// animation.
    fn finalize_overlay_session(
        &mut self,
        final_positions: &[(IconId, Point)],
    ) -> Result<FinalCommitOutcome, DesktopError>;

    /// Render one frame of the overlay off-screen and return the raw
    /// pixels — never touches an `HWND`, DirectComposition target, or
    /// the real desktop. Safe to call at any time, including while a
    /// [`begin_overlay_session`](Self::begin_overlay_session) is
    /// active on another animation.
    ///
    /// `positions` are in **overlay-local pixel coordinates** — the
    /// caller is responsible for translating from virtual-screen
    /// coordinates if needed (subtract the overlay's origin). Only
    /// icons whose id is present in `positions` are drawn; ids not
    /// found in the backend's caches (never `list_icons`'d, or the
    /// icon vanished) render as placeholder tiles.
    ///
    /// `dpi_scale` is the scale factor to render at (`1.0` → 96 DPI,
    /// `2.5` → 240 DPI). Icon-bitmap sizing follows the
    /// backend-supplied `IconRenderPlan::size_px`; the DPI setting
    /// only affects DirectWrite text hinting.
    ///
    /// Default implementation returns
    /// [`DesktopError::OverlayUnavailable`] — non-rendering backends
    /// (`FakeBackend`, `StubBackend`) inherit this.
    fn render_overlay_snapshot(
        &mut self,
        width_px: u32,
        height_px: u32,
        dpi_scale: f32,
        positions: &[(IconId, Point)],
        render_options: OverlayRenderOptions,
    ) -> Result<SnapshotFrame, DesktopError> {
        let _ = (width_px, height_px, dpi_scale, positions, render_options);
        Err(DesktopError::OverlayUnavailable(
            "render_overlay_snapshot is not implemented for this backend".into(),
        ))
    }
}

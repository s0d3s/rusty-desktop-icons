//! An in-memory [`DesktopBackend`] implementation
//! for unit and integration tests.
//!
//! The backend is `Clone`-able (all internal state lives behind
//! `Arc<Mutex<_>>`), so a test can hold one clone for observation while
//! handing another clone to the [`DesktopController`](crate::DesktopController).
//!
//! ```
//! use rdi_core::fake::FakeDesktop;
//! use rdi_core::{IconId, IconSnapshot, Point};
//!
//! let desktop = FakeDesktop::new();
//! desktop.add_icon(IconSnapshot::new(
//!     IconId::from("a"), "A", None, false, Point::new(0, 0),
//! ));
//!
//! // Handing a clone to the controller (M2 wiring covered elsewhere).
//! let backend = desktop.clone();
//! # let _ = backend;
//!
//! // The test can still inspect state:
//! assert_eq!(desktop.icon_count(), 1);
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::backend::DesktopBackend;
use crate::{
    DesktopError, FinalCommitOutcome, IconId, IconRenderPlan, IconSnapshot, MonitorInfo,
    OverlayRenderOptions, Point, Rect,
};

/// Observable, cloneable fake desktop backed by a `HashMap`.
#[derive(Clone, Debug, Default)]
pub struct FakeDesktop {
    inner: Arc<Mutex<FakeDesktopInner>>,
}

#[derive(Debug)]
pub struct FakeDesktopInner {
    /// All icons currently on the fake desktop.
    pub icons: HashMap<IconId, IconSnapshot>,
    /// Current folder-view flags word.
    pub flags: u32,
    /// Log of every `set_positions` batch, in order. Each entry is the
    /// list of `(id, new_pos)` pairs that were actually applied (missing
    /// ids are excluded).
    pub commit_log: Vec<Vec<(IconId, Point)>>,
    /// Log of every `apply_flags(mask, values)` call.
    pub flag_ops: Vec<(u32, u32)>,
    /// If set, `list_icons` will fail with this error the next time it is
    /// called. Cleared after the failing call.
    pub next_list_error: Option<DesktopError>,
    /// Simulated monitor layout. Defaults to a single 1920x1080 primary
    /// monitor at the origin so tests that don't care about multi-
    /// monitor behaviour keep working.
    pub monitors: Vec<MonitorInfo>,
    pub desktop_info: Option<crate::DesktopInfo>,

    // --- overlay-session observability --------------------
    /// If set, `begin_overlay_session` will fail with this error the
    /// next time it is called. Cleared after the failing call.
    pub next_overlay_error: Option<DesktopError>,
    /// Whether an overlay session is currently open on this fake
    /// backend.
    pub overlay_active: bool,
    pub real_icons_visible: bool,
    /// The `IconRenderPlan` list passed to the most recent successful
    /// `begin_overlay_session` call.
    pub last_overlay_plans: Vec<IconRenderPlan>,
    /// The [`OverlayRenderOptions`] passed to the most recent
    /// successful `begin_overlay_session` call. Initialised to
    /// [`OverlayRenderOptions::all_enabled`] until a session opens.
    pub last_overlay_render_options: OverlayRenderOptions,
    /// Log of every `commit_overlay_frame` call, in order.
    pub overlay_frame_log: Vec<Vec<(IconId, Point)>>,
    pub visual_frame_log: Vec<Vec<crate::IconFrame>>,
    /// Log of every `finalize_overlay_session` call — the final
    /// positions committed to the Shell.
    pub overlay_finalize_log: Vec<Vec<(IconId, Point)>>,
    /// Queue of pending errors to return from the next
    /// `commit_overlay_frame` calls (one per call, in order).
    /// Populated by tests via [`FakeDesktop::queue_commit_error`]
    /// so the engine's cancellation / error branches can be
    /// exercised deterministically.
    pub next_commit_errors: std::collections::VecDeque<DesktopError>,
}

impl Default for FakeDesktopInner {
    fn default() -> Self {
        Self {
            icons: HashMap::new(),
            flags: 0,
            commit_log: Vec::new(),
            flag_ops: Vec::new(),
            next_list_error: None,
            monitors: vec![MonitorInfo {
                id: "FAKE-PRIMARY".into(),
                name: "FAKE-PRIMARY".into(),
                bounds: Rect::from_origin_size(0, 0, 1920, 1080),
                work_area: Rect::from_origin_size(0, 0, 1920, 1040),
                is_primary: true,
                scale_factor: 1.0,
            }],
            next_overlay_error: None,
            desktop_info: None,
            overlay_active: false,
            real_icons_visible: true,
            last_overlay_plans: Vec::new(),
            last_overlay_render_options: OverlayRenderOptions::all_enabled(),
            overlay_frame_log: Vec::new(),
            visual_frame_log: Vec::new(),
            overlay_finalize_log: Vec::new(),
            next_commit_errors: std::collections::VecDeque::new(),
        }
    }
}

impl FakeDesktop {
    pub fn real_icons_visible(&self) -> bool {
        self.inner.lock().expect("FakeDesktop poisoned").real_icons_visible
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// Add or replace an icon.
    pub fn add_icon(&self, snap: IconSnapshot) {
        let mut guard = self.inner.lock().expect("FakeDesktop poisoned");
        guard.icons.insert(snap.id.clone(), snap);
    }

    /// Remove an icon (simulates the user deleting it while an animation
    /// is running).
    pub fn remove_icon(&self, id: &IconId) -> bool {
        self.inner
            .lock()
            .expect("FakeDesktop poisoned")
            .icons
            .remove(id)
            .is_some()
    }

    pub fn icon_count(&self) -> usize {
        self.inner.lock().expect("FakeDesktop poisoned").icons.len()
    }

    /// Look up the current position of one icon.
    pub fn position_of(&self, id: &IconId) -> Option<Point> {
        self.inner
            .lock()
            .expect("FakeDesktop poisoned")
            .icons
            .get(id)
            .map(|s| s.position)
    }

    /// Current folder-view flags.
    pub fn flags(&self) -> u32 {
        self.inner.lock().expect("FakeDesktop poisoned").flags
    }

    /// Take a copy of the commit log so far. Does not clear it.
    pub fn commit_log(&self) -> Vec<Vec<(IconId, Point)>> {
        self.inner
            .lock()
            .expect("FakeDesktop poisoned")
            .commit_log
            .clone()
    }

    /// Take a copy of the flag-op log so far.
    pub fn flag_ops(&self) -> Vec<(u32, u32)> {
        self.inner
            .lock()
            .expect("FakeDesktop poisoned")
            .flag_ops
            .clone()
    }

    /// Cause the next `list_icons` call to fail with this error.
    pub fn set_next_list_error(&self, err: DesktopError) {
        self.inner
            .lock()
            .expect("FakeDesktop poisoned")
            .next_list_error = Some(err);
    }

    /// Replace the simulated monitor layout. Useful for testing
    /// multi-monitor placement without needing a real second display.
    pub fn set_monitors(&self, monitors: Vec<MonitorInfo>) {
        self.inner
            .lock()
            .expect("FakeDesktop poisoned")
            .monitors = monitors;
    }

    pub fn set_desktop_info(&self, desktop: crate::DesktopInfo) {
        self.inner.lock().expect("FakeDesktop poisoned").desktop_info = Some(desktop);
    }

    /// Cause the next `begin_overlay_session` call to fail with this
    /// error. Used by tests to exercise the engine's overlay-unavailable
    /// fallback path.
    pub fn set_next_overlay_error(&self, err: DesktopError) {
        self.inner
            .lock()
            .expect("FakeDesktop poisoned")
            .next_overlay_error = Some(err);
    }

    /// Whether an overlay session is currently open.
    pub fn overlay_active(&self) -> bool {
        self.inner
            .lock()
            .expect("FakeDesktop poisoned")
            .overlay_active
    }

    /// Snapshot of the plans handed to the most recent successful
    /// `begin_overlay_session` call.
    pub fn last_overlay_plans(&self) -> Vec<IconRenderPlan> {
        self.inner
            .lock()
            .expect("FakeDesktop poisoned")
            .last_overlay_plans
            .clone()
    }

    /// Snapshot of the render options handed to the most recent
    /// successful `begin_overlay_session` call. Defaults to
    /// `all_enabled` before any session has opened.
    pub fn last_overlay_render_options(&self) -> OverlayRenderOptions {
        self.inner
            .lock()
            .expect("FakeDesktop poisoned")
            .last_overlay_render_options
    }

    /// Snapshot of every `commit_overlay_frame` call in order.
    pub fn overlay_frame_log(&self) -> Vec<Vec<(IconId, Point)>> {
        self.inner
            .lock()
            .expect("FakeDesktop poisoned")
            .overlay_frame_log
            .clone()
    }

    pub fn visual_frame_log(&self) -> Vec<Vec<crate::IconFrame>> {
        self.inner.lock().expect("FakeDesktop poisoned").visual_frame_log.clone()
    }

    /// Snapshot of every `finalize_overlay_session` call in order.
    pub fn overlay_finalize_log(&self) -> Vec<Vec<(IconId, Point)>> {
        self.inner
            .lock()
            .expect("FakeDesktop poisoned")
            .overlay_finalize_log
            .clone()
    }

    /// Queue an error to be returned from the next
    /// `commit_overlay_frame` call. Multiple queued errors are
    /// consumed in FIFO order.
    ///
    /// Useful for exercising the engine's
    /// [`DesktopError::OverlayCancelled`] path from tests.
    pub fn queue_commit_error(&self, err: DesktopError) {
        self.inner
            .lock()
            .expect("FakeDesktop poisoned")
            .next_commit_errors
            .push_back(err);
    }
}

impl DesktopBackend for FakeDesktop {
    fn list_icons(&mut self) -> Result<Vec<IconSnapshot>, DesktopError> {
        let mut guard = self.inner.lock().expect("FakeDesktop poisoned");
        if let Some(err) = guard.next_list_error.take() {
            return Err(err);
        }
        Ok(guard.icons.values().cloned().collect())
    }

    fn get_flags(&mut self) -> Result<u32, DesktopError> {
        Ok(self.inner.lock().expect("FakeDesktop poisoned").flags)
    }

    fn apply_flags(&mut self, mask: u32, values: u32) -> Result<(), DesktopError> {
        let mut guard = self.inner.lock().expect("FakeDesktop poisoned");
        guard.flag_ops.push((mask, values));
        guard.flags = (guard.flags & !mask) | (values & mask);
        Ok(())
    }

    fn set_positions(
        &mut self,
        moves: &[(IconId, Point)],
    ) -> Result<Vec<IconId>, DesktopError> {
        let mut guard = self.inner.lock().expect("FakeDesktop poisoned");
        let mut applied = Vec::with_capacity(moves.len());
        let mut missing = Vec::new();
        for (id, pt) in moves {
            if let Some(snap) = guard.icons.get_mut(id) {
                snap.position = *pt;
                applied.push((id.clone(), *pt));
            } else {
                missing.push(id.clone());
            }
        }
        guard.commit_log.push(applied);
        Ok(missing)
    }

    fn list_monitors(&mut self) -> Result<Vec<MonitorInfo>, DesktopError> {
        Ok(self.inner.lock().expect("FakeDesktop poisoned").monitors.clone())
    }

    fn desktop_info(&mut self, _icons: &[IconSnapshot]) -> Result<crate::DesktopInfo, DesktopError> {
        self.inner.lock().expect("FakeDesktop poisoned").desktop_info.clone()
            .ok_or(DesktopError::UnsupportedPlatform)
    }

    fn begin_overlay_session(
        &mut self,
        plans: &[IconRenderPlan],
        render_options: OverlayRenderOptions,
    ) -> Result<(), DesktopError> {
        let mut guard = self.inner.lock().expect("FakeDesktop poisoned");
        if let Some(err) = guard.next_overlay_error.take() {
            return Err(err);
        }
        guard.overlay_active = true;
        guard.real_icons_visible = false;
        guard.last_overlay_plans = plans.to_vec();
        guard.last_overlay_render_options = render_options;
        Ok(())
    }

    fn commit_overlay_frame(
        &mut self,
        positions: &[(IconId, Point)],
    ) -> Result<(), DesktopError> {
        let mut guard = self.inner.lock().expect("FakeDesktop poisoned");
        if let Some(err) = guard.next_commit_errors.pop_front() {
            return Err(err);
        }
        guard.overlay_frame_log.push(positions.to_vec());
        Ok(())
    }

    fn discard_overlay_session(&mut self) {
        let mut guard = self.inner.lock().expect("FakeDesktop poisoned");
        guard.overlay_active = false;
        guard.real_icons_visible = true;
    }

    fn commit_visual_frame(&mut self, frame: &[crate::IconFrame]) -> Result<(), DesktopError> {
        self.commit_overlay_frame(&frame.iter().map(|entry| (entry.id.clone(), entry.position)).collect::<Vec<_>>())?;
        self.inner.lock().expect("FakeDesktop poisoned").visual_frame_log.push(frame.to_vec());
        Ok(())
    }

    fn poll_overlay_session(&mut self) -> Result<(), DesktopError> {
        match self.inner.lock().expect("FakeDesktop poisoned").next_commit_errors.pop_front() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn set_real_icons_visible(&mut self, visible: bool) -> Result<(), DesktopError> {
        let mut guard = self.inner.lock().expect("FakeDesktop poisoned");
        if !guard.overlay_active {
            return Err(DesktopError::BackendUnavailable("no active overlay".into()));
        }
        guard.real_icons_visible = visible;
        Ok(())
    }

    fn finalize_overlay_session(
        &mut self,
        final_positions: &[(IconId, Point)],
    ) -> Result<FinalCommitOutcome, DesktopError> {
        let mut guard = self.inner.lock().expect("FakeDesktop poisoned");
        guard.overlay_finalize_log.push(final_positions.to_vec());
        guard.overlay_active = false;
        guard.real_icons_visible = true;

        // Apply the moves to the fake icon storage — this is what a
        // real backend achieves via `SelectAndPositionItems`. Track
        // moved / missing separately for parity with the trait doc.
        let mut moved = Vec::with_capacity(final_positions.len());
        let mut missing = Vec::new();
        for (id, pt) in final_positions {
            match guard.icons.get_mut(id) {
                Some(snap) => {
                    snap.position = *pt;
                    moved.push(id.clone());
                }
                None => missing.push(id.clone()),
            }
        }
        Ok(FinalCommitOutcome {
            moved_ids: moved,
            missing_ids: missing,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(id: &str, x: i32, y: i32) -> IconSnapshot {
        IconSnapshot::new(IconId::from(id), id, None, false, Point::new(x, y))
    }

    #[test]
    fn apply_flags_mask_semantics_match_shell() {
        let d = FakeDesktop::new();
        let mut b = d.clone();
        // Start from 0b0101.
        b.apply_flags(0b1111, 0b0101).unwrap();
        assert_eq!(d.flags(), 0b0101);

        // OR-set (legacy set_desktop_flags): mask = f, values = 0xFFFF_FFFF.
        b.apply_flags(0b0010, 0xFFFF_FFFF).unwrap();
        assert_eq!(d.flags(), 0b0111);

        // AND-clear (legacy unset_desktop_flags): mask = f, values = 0.
        b.apply_flags(0b0100, 0).unwrap();
        assert_eq!(d.flags(), 0b0011);
    }

    #[test]
    fn set_positions_updates_and_reports_missing() {
        let d = FakeDesktop::new();
        d.add_icon(snap("a", 0, 0));
        d.add_icon(snap("b", 10, 10));

        let mut b = d.clone();
        let missing = b
            .set_positions(&[
                (IconId::from("a"), Point::new(1, 2)),
                (IconId::from("ghost"), Point::new(0, 0)),
            ])
            .unwrap();

        assert_eq!(missing, vec![IconId::from("ghost")]);
        assert_eq!(d.position_of(&IconId::from("a")), Some(Point::new(1, 2)));
        assert_eq!(d.position_of(&IconId::from("b")), Some(Point::new(10, 10)));
        let log = d.commit_log();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0], vec![(IconId::from("a"), Point::new(1, 2))]);
    }

    #[test]
    fn list_icons_returns_error_when_armed() {
        let d = FakeDesktop::new();
        d.set_next_list_error(DesktopError::BackendUnavailable("bang".into()));
        let mut b = d.clone();
        assert!(matches!(
            b.list_icons(),
            Err(DesktopError::BackendUnavailable(_))
        ));
        // Second call succeeds again (error consumed).
        assert!(b.list_icons().is_ok());
    }
}

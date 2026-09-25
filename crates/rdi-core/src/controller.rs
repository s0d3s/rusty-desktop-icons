//! Public [`DesktopController`] — the top-level entry point for the crate.
//!
//! A `DesktopController` owns exactly one worker thread and one
//! [`DesktopBackend`]. All public methods forward
//! their requests to that worker and (for synchronous calls) block until
//! it replies.

use std::sync::Mutex;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{bounded, unbounded, Sender};

use crate::backend::DesktopBackend;
use crate::engine::{worker_main, WorkerMsg};
use crate::handle::{AnimationHandle, PreObservers};
use crate::spec::{AnimationOptions, IconAnimationSpec};
use crate::{DesktopError, IconId, IconSnapshot, MonitorInfo, OverlayRenderOptions, Point, SnapshotFrame};

/// Top-level entry point.
///
/// Cheap `Clone` is deliberately **not** implemented; sharing across
/// threads is done via `Arc<DesktopController>` on the caller side. The
/// worker thread is joined on `Drop`.
pub struct DesktopController {
    tx: Sender<WorkerMsg>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

/// One-shot reservation of a prepared animation on its controller's worker.
/// Dropping it before start cancels without moving desktop icons.
pub struct PreparedAnimation {
    trigger: Sender<crate::engine::StartMode>,
    handle: AnimationHandle,
}

impl PreparedAnimation {
    /// Consume the reservation and start its visual clock on the worker.
    pub fn start(self) -> Result<AnimationHandle, DesktopError> {
        self.trigger.send(crate::engine::StartMode::Animation).map_err(|_| DesktopError::WorkerCrashed("preparation expired".into()))?;
        Ok(self.handle)
    }

    pub fn open_timeline(self) -> Result<crate::TimelineSession, DesktopError> {
        let (session, runtime) = crate::TimelineSession::pair(self.handle.clone());
        let (ready, response) = bounded(1);
        self.trigger.send(crate::engine::StartMode::Timeline { runtime, ready })
            .map_err(|_| DesktopError::BackendUnavailable("preparation expired".into()))?;
        response.recv().map_err(|_| DesktopError::BackendUnavailable(
            format!("timeline opening failed: {:?}", self.handle.wait())
        ))?;
        Ok(session)
    }

    /// Discard GPU resources and wait for the reservation to be released.
    pub fn cancel(self) {
        let _ = self.trigger.send(crate::engine::StartMode::Cancel);
        self.handle.wait();
    }
}

impl DesktopController {
    /// Spawn a worker thread that owns `backend` and start serving
    /// requests.
    pub fn new<B: DesktopBackend>(backend: B) -> Result<Self, DesktopError> {
        Self::from_boxed(Box::new(backend))
    }

    /// Same as [`Self::new`] but takes an already-boxed backend, useful
    /// when the backend type is only known dynamically.
    pub fn from_boxed(backend: Box<dyn DesktopBackend>) -> Result<Self, DesktopError> {
        let (tx, rx) = unbounded::<WorkerMsg>();
        let tx_for_worker = tx.clone();
        let worker = thread::Builder::new()
            .name("rdi-worker".into())
            .spawn(move || worker_main(tx_for_worker, rx, backend))
            .map_err(|e| {
                DesktopError::BackendUnavailable(format!("failed to spawn worker thread: {e}"))
            })?;
        Ok(Self {
            tx,
            worker: Mutex::new(Some(worker)),
        })
    }

    // ---- simple sync operations -----------------------------------------

    /// Snapshot every icon currently on the desktop.
    pub fn list_icons(&self) -> Result<Vec<IconSnapshot>, DesktopError> {
        let (rtx, rrx) = bounded(1);
        self.tx
            .send(WorkerMsg::ListIcons { resp: rtx })
            .map_err(|_| DesktopError::WorkerCrashed("channel closed".into()))?;
        rrx.recv()
            .map_err(|_| DesktopError::WorkerCrashed("worker dropped response".into()))?
    }

    /// Read the raw desktop folder-view flag word.
    pub fn get_flags(&self) -> Result<u32, DesktopError> {
        let (rtx, rrx) = bounded(1);
        self.tx
            .send(WorkerMsg::GetFlags { resp: rtx })
            .map_err(|_| DesktopError::WorkerCrashed("channel closed".into()))?;
        rrx.recv()
            .map_err(|_| DesktopError::WorkerCrashed("worker dropped response".into()))?
    }

    /// Perform a masked update of the desktop folder-view flags.
    ///
    /// `new_flags = (old_flags & !mask) | (values & mask)`.
    ///
    /// Returns [`DesktopError::AnimationBusy`] while an animation is in
    /// flight — a mid-animation flag write can un-hide the real icons
    /// behind the overlay. Use [`AnimationOptions::before_flags`] /
    /// [`after_flags`](AnimationOptions::after_flags) to bracket an
    /// animation with flag changes.
    pub fn apply_flags(&self, mask: u32, values: u32) -> Result<(), DesktopError> {
        let (rtx, rrx) = bounded(1);
        self.tx
            .send(WorkerMsg::ApplyFlags {
                mask,
                values,
                resp: rtx,
            })
            .map_err(|_| DesktopError::WorkerCrashed("channel closed".into()))?;
        rrx.recv()
            .map_err(|_| DesktopError::WorkerCrashed("worker dropped response".into()))?
    }

    /// OR-set semantics — matches the legacy `set_desktop_flags(flags)`.
    #[inline]
    pub fn set_flags(&self, flags: u32) -> Result<(), DesktopError> {
        self.apply_flags(flags, 0xFFFF_FFFF)
    }

    /// AND-clear semantics — matches the legacy `unset_desktop_flags(flags)`.
    #[inline]
    pub fn unset_flags(&self, flags: u32) -> Result<(), DesktopError> {
        self.apply_flags(flags, 0)
    }

    /// XOR-toggle semantics — matches the legacy `switch_desktop_flags(flags)`.
    pub fn toggle_flags(&self, flags: u32) -> Result<(), DesktopError> {
        let current = self.get_flags()?;
        self.apply_flags(flags, current ^ flags)
    }

    /// Exactly-set semantics — matches the legacy `exactly_set_desktop_flags(flags)`.
    pub fn set_flags_exactly(&self, flags: u32) -> Result<(), DesktopError> {
        self.apply_flags(crate::engine::build_true_mask(flags), flags)
    }

    /// Instantly move a batch of icons to the specified positions
    /// (no animation). Returns the ids the backend could not resolve.
    ///
    /// Returns [`DesktopError::AnimationBusy`] while an animation is in
    /// flight — the positions would be overwritten by the animation's
    /// final commit. Stop the animation first.
    pub fn set_positions(
        &self,
        moves: Vec<(IconId, Point)>,
    ) -> Result<Vec<IconId>, DesktopError> {
        let (rtx, rrx) = bounded(1);
        self.tx
            .send(WorkerMsg::SetPositions { moves, resp: rtx })
            .map_err(|_| DesktopError::WorkerCrashed("channel closed".into()))?;
        rrx.recv()
            .map_err(|_| DesktopError::WorkerCrashed("worker dropped response".into()))?
    }

    /// Enumerate every connected display in virtual-screen
    /// coordinates.
    ///
    /// The returned [`MonitorInfo`] entries share the same coordinate
    /// system as icon positions, so a caller can decide "put this icon
    /// on the second monitor" by checking `monitor.bounds` and building
    /// a `Point` inside those bounds.
    pub fn list_monitors(&self) -> Result<Vec<MonitorInfo>, DesktopError> {
        let (rtx, rrx) = bounded(1);
        self.tx
            .send(WorkerMsg::ListMonitors { resp: rtx })
            .map_err(|_| DesktopError::WorkerCrashed("channel closed".into()))?;
        rrx.recv()
            .map_err(|_| DesktopError::WorkerCrashed("worker dropped response".into()))?
    }

    /// Read current screen and icon-grid metrics, including during playback.
    pub fn desktop_info(&self) -> Result<crate::DesktopInfo, DesktopError> {
        let (response, receiver) = bounded(1);
        self.tx.send(WorkerMsg::DesktopInfo { resp: response })
            .map_err(|_| DesktopError::WorkerCrashed("channel closed".into()))?;
        receiver.recv()
            .map_err(|_| DesktopError::WorkerCrashed("worker dropped response".into()))?
    }

    /// Render one frame of the overlay off-screen and return the raw
    /// pixel buffer + dimensions. Never touches an on-screen window,
    /// DirectComposition, the real desktop, or the display mode.
    ///
    /// `positions` are in **overlay-local pixel coordinates** (top-left
    /// origin). `dpi_scale` is the scale factor to render at
    /// (`1.0` → 96 DPI, `2.5` → 240 DPI).
    ///
    /// The backend must have been populated with icon metadata by a
    /// prior [`Self::list_icons`] call for icon bitmaps + labels to
    /// appear; ids missing from the backend's caches render as
    /// placeholder tiles.
    ///
    /// Returns [`DesktopError::OverlayUnavailable`] on backends that
    /// do not implement rendering (the cross-platform stub, the
    /// test fake, or platforms other than Windows).
    pub fn render_overlay_snapshot(
        &self,
        width_px: u32,
        height_px: u32,
        dpi_scale: f32,
        positions: Vec<(IconId, Point)>,
        render_options: OverlayRenderOptions,
    ) -> Result<SnapshotFrame, DesktopError> {
        let (rtx, rrx) = bounded(1);
        self.tx
            .send(WorkerMsg::RenderOverlaySnapshot {
                width_px,
                height_px,
                dpi_scale,
                positions,
                render_options,
                resp: rtx,
            })
            .map_err(|_| DesktopError::WorkerCrashed("channel closed".into()))?;
        rrx.recv()
            .map_err(|_| DesktopError::WorkerCrashed("worker dropped response".into()))?
    }

    // ---- animation ------------------------------------------------------

    pub fn prepare_scene(&self, scene: crate::Scene) -> Result<crate::RenderSession, DesktopError> {
        let (resp, response) = bounded(1);
        self.tx.send(WorkerMsg::PrepareScene { scene, resp })
            .map_err(|_| DesktopError::WorkerCrashed("channel closed".into()))?;
        response.recv().map_err(|_| DesktopError::WorkerCrashed("worker dropped response".into()))?
    }

    /// Build overlay resources now without displaying or moving anything.
    /// The snapshot is fixed at preparation; reprepare after desktop changes.
    pub fn prepare(
        &self,
        specs: Vec<IconAnimationSpec>,
        options: AnimationOptions,
    ) -> Result<PreparedAnimation, DesktopError> {
        let (trigger, start) = bounded(1);
        let (ready, readiness) = bounded(1);
        let (resp, response) = bounded(1);
        self.tx.send(WorkerMsg::StartAnimation {
            specs, options, observers: PreObservers::default(), resp,
            preparation: Some((ready, start)),
        }).map_err(|_| DesktopError::WorkerCrashed("channel closed".into()))?;
        let handle = response.recv().map_err(|_| DesktopError::WorkerCrashed("worker dropped response".into()))??;
        if readiness.recv().is_err() {
            return Err(DesktopError::BackendUnavailable(format!("preparation failed: {:?}", handle.wait())));
        }
        Ok(PreparedAnimation { trigger, handle })
    }

    /// Start an animation.
    ///
    /// Returns immediately with a handle. Use [`AnimationHandle::wait`]
    /// to block until completion, or the various `on_*` observers for
    /// non-blocking notification.
    ///
    /// **Observer race warning:** callbacks attached via
    /// [`AnimationHandle::on_start`] / `on_tick` / `on_icon_complete` /
    /// `on_finish` are inherently racy — the worker may already be
    /// ticking (and firing events) by the time your registration lands.
    /// If precise counts matter, use
    /// [`Self::animate_with_observers`] instead, which installs the
    /// callbacks *before* the worker enters its tick loop.
    pub fn animate(
        &self,
        specs: Vec<IconAnimationSpec>,
        options: AnimationOptions,
    ) -> Result<AnimationHandle, DesktopError> {
        self.animate_with_observers(specs, options, PreObservers::default())
    }

    /// Same as [`Self::animate`] but atomically pre-attaches a bundle of
    /// observers, guaranteeing that every event fired by the worker is
    /// delivered to those callbacks.
    pub fn animate_with_observers(
        &self,
        specs: Vec<IconAnimationSpec>,
        options: AnimationOptions,
        observers: PreObservers,
    ) -> Result<AnimationHandle, DesktopError> {
        let (rtx, rrx) = bounded(1);
        self.tx
            .send(WorkerMsg::StartAnimation {
                specs,
                options,
                observers,
                resp: rtx,
                preparation: None,
            })
            .map_err(|_| DesktopError::WorkerCrashed("channel closed".into()))?;
        rrx.recv()
            .map_err(|_| DesktopError::WorkerCrashed("worker dropped response".into()))?
    }

    // ---- lifecycle ------------------------------------------------------

    /// Stop the worker thread and block until it has exited.
    ///
    /// Idempotent, and called by [`Drop`]. Exposed separately so
    /// embedders that must not block on the calling thread's own
    /// resources — notably PyO3, which runs `Drop` with the GIL held
    /// while the worker may be waiting to *acquire* the GIL inside an
    /// observer callback — can perform the join at a point of their
    /// choosing.
    pub fn shutdown(&self) {
        let _ = self.tx.send(WorkerMsg::Shutdown);
        if let Ok(mut guard) = self.worker.lock() {
            if let Some(handle) = guard.take() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for DesktopController {
    fn drop(&mut self) {
        self.shutdown();
    }
}

//! Public [`AnimationHandle`] and the shared state it exposes.
//!
//! The handle is what user code manipulates while an animation is running.
//! Internally it wraps an `Arc<HandleInner>` shared with the worker thread
//! so both sides can update / observe the same state.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration as StdDuration, Instant};

use crossbeam_channel::Sender;

use crate::engine::WorkerMsg;
use crate::events::{
    FinishReason, IconAnimationState, StartContext, StopMode, TickContext,
};
use crate::{FinalCommitOutcome, IconId};

// ---------------------------------------------------------------------------
// Observer callback types
// ---------------------------------------------------------------------------

pub(crate) type ObserverStart = Arc<dyn Fn(&StartContext) + Send + Sync>;
pub(crate) type ObserverTick = Arc<dyn Fn(&TickContext) + Send + Sync>;
pub(crate) type ObserverIconComplete = Arc<dyn Fn(&IconId) + Send + Sync>;
pub(crate) type ObserverFinish = Arc<dyn Fn(&FinishReason) + Send + Sync>;

#[derive(Default)]
pub(crate) struct Observers {
    pub on_start: Vec<ObserverStart>,
    pub on_tick: Vec<ObserverTick>,
    pub on_icon_complete: Vec<ObserverIconComplete>,
    pub on_finish: Vec<ObserverFinish>,
}

/// Bundle of observers to attach *atomically* to a new animation.
///
/// [`AnimationHandle::on_*`](AnimationHandle) can only be called after the
/// handle is returned, which races against the worker's first tick — a
/// completion that fires before your `on_icon_complete` callback is
/// registered will simply be lost. Passing observers through this bundle
/// via [`DesktopController::animate_with_observers`](crate::DesktopController::animate_with_observers)
/// installs them *before* the worker enters its tick loop.
///
/// ```
/// # use rdi_core::PreObservers;
/// let obs = PreObservers::new()
///     .on_tick(|ctx| println!("tick #{}", ctx.finalized_icons))
///     .on_finish(|reason| println!("finished: {reason:?}"));
/// # let _ = obs;
/// ```
#[derive(Default, Clone)]
pub struct PreObservers {
    pub(crate) on_start: Vec<ObserverStart>,
    pub(crate) on_tick: Vec<ObserverTick>,
    pub(crate) on_icon_complete: Vec<ObserverIconComplete>,
    pub(crate) on_finish: Vec<ObserverFinish>,
}

impl PreObservers {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn on_start<F>(mut self, cb: F) -> Self
    where
        F: Fn(&StartContext) + Send + Sync + 'static,
    {
        self.on_start.push(Arc::new(cb));
        self
    }

    #[must_use]
    pub fn on_tick<F>(mut self, cb: F) -> Self
    where
        F: Fn(&TickContext) + Send + Sync + 'static,
    {
        self.on_tick.push(Arc::new(cb));
        self
    }

    #[must_use]
    pub fn on_icon_complete<F>(mut self, cb: F) -> Self
    where
        F: Fn(&IconId) + Send + Sync + 'static,
    {
        self.on_icon_complete.push(Arc::new(cb));
        self
    }

    #[must_use]
    pub fn on_finish<F>(mut self, cb: F) -> Self
    where
        F: Fn(&FinishReason) + Send + Sync + 'static,
    {
        self.on_finish.push(Arc::new(cb));
        self
    }
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

#[derive(Default)]
pub(crate) struct HandleState {
    pub running: bool,
    pub progress: f32,
    pub per_icon: Vec<IconAnimationState>,
    pub missing: Vec<IconId>,
    pub finish_reason: Option<FinishReason>,
    /// Result of the Shell-side final commit, once
    /// `finalize_overlay_session` has returned. `None` while the
    /// animation is still running, on the overlay-unavailable fallback
    /// path, and when finalization itself failed.
    pub final_commit: Option<FinalCommitOutcome>,
}

pub(crate) struct HandleInner {
    /// Sender for control messages ([`WorkerMsg::AnimStop`], …). Cloned
    /// from the controller's tx when the animation starts.
    pub(crate) ctrl_tx: Sender<WorkerMsg>,
    pub(crate) state: Mutex<HandleState>,
    pub(crate) observers: Mutex<Observers>,
    pub(crate) finish_cv: Condvar,
}

impl HandleInner {
    pub(crate) fn new(ctrl_tx: Sender<WorkerMsg>) -> Arc<Self> {
        Arc::new(Self {
            ctrl_tx,
            // Start in the "running" state so that a caller who immediately
            // calls `wait` / `wait_timeout` on the returned handle does not
            // observe a spurious "already finished" state before the worker
            // has had a chance to enter its tick loop. The worker will flip
            // `running` back to `false` when the animation truly ends
            // (either via the tick loop's normal termination or through
            // `finalize_with_error`).
            state: Mutex::new(HandleState {
                running: true,
                progress: 0.0,
                per_icon: Vec::new(),
                missing: Vec::new(),
                finish_reason: None,
                final_commit: None,
            }),
            observers: Mutex::new(Observers::default()),
            finish_cv: Condvar::new(),
        })
    }

    /// Clone the current observer list under a short lock, so the caller
    /// can fire callbacks without holding the state lock.
    pub(crate) fn observers_snapshot(&self) -> Observers {
        let g = self.lock_observers();
        Observers {
            on_start: g.on_start.clone(),
            on_tick: g.on_tick.clone(),
            on_icon_complete: g.on_icon_complete.clone(),
            on_finish: g.on_finish.clone(),
        }
    }

    /// Move the pre-configured observers from a [`PreObservers`] bundle
    /// into the shared observer list. Called by the worker *before* it
    /// hands the handle back to the caller so the observers are visible
    /// on the very first tick.
    pub(crate) fn install_pre_observers(&self, pre: PreObservers) {
        let mut g = self.lock_observers();
        g.on_start.extend(pre.on_start);
        g.on_tick.extend(pre.on_tick);
        g.on_icon_complete.extend(pre.on_icon_complete);
        g.on_finish.extend(pre.on_finish);
    }

    /// Lock [`Self::state`], recovering from poisoning.
    ///
    /// A poisoned lock here means the worker panicked mid-animation.
    /// `HandleState` is plain data — progress numbers, a reason enum,
    /// per-icon snapshots — so there is no half-updated invariant a
    /// panic could have left behind, and the worst a reader sees is a
    /// stale value. Propagating the poison instead would turn one
    /// worker panic into a permanently-panicking handle: every later
    /// `progress()` / `wait()` / `snapshot()` would panic too, and
    /// through PyO3 that surfaces as `PanicException` with no way back.
    #[inline]
    pub(crate) fn lock_state(&self) -> MutexGuard<'_, HandleState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Lock [`Self::observers`], recovering from poisoning. Same
    /// rationale as [`Self::lock_state`] — the observer lists are
    /// `Vec<Arc<dyn Fn>>` with no cross-field invariant.
    #[inline]
    pub(crate) fn lock_observers(&self) -> MutexGuard<'_, Observers> {
        self.observers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

// ---------------------------------------------------------------------------
// Public handle
// ---------------------------------------------------------------------------

/// Handle returned by [`DesktopController::animate`](crate::DesktopController::animate).
///
/// The handle is `Clone`-able and thread-safe: user code can hand copies
/// to different threads (e.g. one thread waiting on completion, another
/// polling progress or issuing `stop`).
#[derive(Clone)]
pub struct AnimationHandle {
    inner: Arc<HandleInner>,
}

impl AnimationHandle {
    pub(crate) fn from_inner(inner: Arc<HandleInner>) -> Self {
        Self { inner }
    }

    /// `true` while the animation is still ticking.
    pub fn is_running(&self) -> bool {
        self.inner.lock_state().running
    }

    /// Global progress `∈ [0, 1]` computed as the mean per-icon `t`.
    pub fn progress(&self) -> f32 {
        self.inner.lock_state().progress
    }

    /// Copy of the current per-icon state.
    pub fn snapshot(&self) -> Vec<IconAnimationState> {
        self.inner.lock_state().per_icon.clone()
    }

    /// Ids that were listed in the spec but were not present on the
    /// desktop at animation start. Populated once, before the first tick.
    pub fn missing_icons(&self) -> Vec<IconId> {
        self.inner.lock_state().missing.clone()
    }

    /// Read the finish reason if the animation has already ended.
    pub fn finish_reason(&self) -> Option<FinishReason> {
        self.inner.lock_state().finish_reason.clone()
    }

    /// Outcome of the Shell-side final commit, available once the
    /// animation has finished.
    ///
    /// `None` means the commit outcome is unknown: the animation is
    /// still running, it took the overlay-unavailable fallback path, or
    /// `finalize_overlay_session` itself failed (in which case
    /// [`Self::finish_reason`] carries the error).
    ///
    /// Read [`FinalCommitOutcome::moved_ids`] carefully — the Windows
    /// backend stops polling at the *first* confirmed icon, so a
    /// non-empty `moved_ids` means "the Shell confirmed the commit
    /// landed", not "here is every icon that moved".
    /// [`FinalCommitOutcome::missing_ids`] is the complete list of ids
    /// the backend could not resolve.
    pub fn final_commit(&self) -> Option<FinalCommitOutcome> {
        self.inner.lock_state().final_commit.clone()
    }

    // -------- animation control -----------------------------------------

    /// Ask the engine to stop the animation. Returns immediately; use
    /// [`Self::wait`] to await the actual finish.
    ///
    /// This is the **only** way to alter a running animation. To change
    /// targets, curves or the icon set, stop, read [`Self::snapshot`] for
    /// the settled positions, and start a fresh
    /// [`animate`](crate::DesktopController::animate).
    pub fn stop(&self, mode: StopMode) {
        let _ = self.inner.ctrl_tx.send(WorkerMsg::AnimStop { mode, owner: Arc::downgrade(&self.inner) });
    }

    // -------- waiting ---------------------------------------------------

    /// Block until the animation finishes and return the finish reason.
    pub fn wait(&self) -> FinishReason {
        let mut state = self.inner.lock_state();
        while state.running {
            state = self
                .inner
                .finish_cv
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        state
            .finish_reason
            .clone()
            .unwrap_or(FinishReason::Completed)
    }

    /// Block for up to `timeout` waiting for the animation to finish.
    /// Returns `None` if the timeout elapsed while the animation was
    /// still running.
    pub fn wait_timeout(&self, timeout: StdDuration) -> Option<FinishReason> {
        let mut state = self.inner.lock_state();
        let deadline = Instant::now() + timeout;
        while state.running {
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            let (new_state, result) = self
                .inner
                .finish_cv
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(PoisonError::into_inner);
            state = new_state;
            if result.timed_out() && state.running {
                return None;
            }
        }
        Some(
            state
                .finish_reason
                .clone()
                .unwrap_or(FinishReason::Completed),
        )
    }

    // -------- observers -------------------------------------------------

    pub fn on_start<F>(&self, cb: F)
    where
        F: Fn(&StartContext) + Send + Sync + 'static,
    {
        self.inner.lock_observers().on_start.push(Arc::new(cb));
    }

    pub fn on_tick<F>(&self, cb: F)
    where
        F: Fn(&TickContext) + Send + Sync + 'static,
    {
        self.inner.lock_observers().on_tick.push(Arc::new(cb));
    }

    pub fn on_icon_complete<F>(&self, cb: F)
    where
        F: Fn(&IconId) + Send + Sync + 'static,
    {
        self.inner
            .lock_observers()
            .on_icon_complete
            .push(Arc::new(cb));
    }

    pub fn on_finish<F>(&self, cb: F)
    where
        F: Fn(&FinishReason) + Send + Sync + 'static,
    {
        self.inner.lock_observers().on_finish.push(Arc::new(cb));
    }
}

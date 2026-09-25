//! Event and mode types passed to observer callbacks and returned from
//! [`AnimationHandle`](crate::AnimationHandle) queries.

use std::time::{Duration as StdDuration, Instant};

use crate::{IconId, Point};

/// How to leave the desktop when [`AnimationHandle::stop`] is invoked.
///
/// [`AnimationHandle::stop`]: crate::AnimationHandle::stop
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StopMode {
    /// Icons keep their current, mid-animation positions.
    LeaveInPlace,
    /// Icons snap to their target positions immediately.
    TeleportToTarget,
}

/// Why an animation ended. Delivered through the observer `on_finish` hook
/// and returned from [`AnimationHandle::wait`].
///
/// [`AnimationHandle::wait`]: crate::AnimationHandle::wait
#[derive(Clone, Debug, PartialEq)]
pub enum FinishReason {
    /// All active icons reached their targets.
    Completed,
    /// The animation was stopped by an explicit
    /// [`AnimationHandle::stop`](crate::AnimationHandle::stop) call.
    Stopped(StopMode),
    /// The backend reported a fatal error mid-animation.
    ///
    /// The string carries a human-readable description of the underlying
    /// [`DesktopError`](crate::DesktopError). The enum itself is not
    /// retained, so `FinishReason` stays cheap to clone and safe to
    /// send across language boundaries.
    Error(String),
}

/// Per-icon state, captured under a snapshot lock.
#[derive(Clone, Debug, PartialEq)]
pub struct IconAnimationState {
    pub id: IconId,
    pub origin: Point,
    pub current: Point,
    pub target: Point,
    /// Normalized time in `[0, 1]` — how far this icon is through its own
    /// animation, independent of the global animation clock.
    pub t: f32,
    /// `true` once the engine has issued the icon's final commit.
    pub is_final: bool,
}

/// Payload for `on_start`.
#[derive(Clone, Debug)]
pub struct StartContext {
    pub total_icons: usize,
    pub missing_icons: usize,
    pub started_at: Instant,
}

/// Payload for `on_tick`.
#[derive(Clone, Debug)]
pub struct TickContext {
    /// Wall-clock time since the animation started.
    pub elapsed: StdDuration,
    /// Global progress `∈ [0, 1]` (average per-icon `t`, weighted equally).
    pub progress: f32,
    /// Icons still moving.
    pub active_icons: usize,
    /// Icons that have already reached their target this animation.
    pub finalized_icons: usize,
}

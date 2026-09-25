//! Worker thread, control-message dispatch and tick loop.
//!
//! The engine owns a single [`DesktopBackend`](crate::DesktopBackend)
//! trait object and runs it on a dedicated OS thread. All commands flow
//! through a crossbeam MPSC channel; during an animation the same channel
//! delivers `AnimStop` plus read-only queries, which are drained between
//! ticks. An animation's active set is fixed at `StartAnimation`.
//!
//! COM apartment threading (Windows) is honoured automatically: the
//! backend is created on this worker thread and never moves.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use tracing::{debug, info, info_span, trace, warn};

use crate::backend::DesktopBackend;
use crate::events::{
    FinishReason, IconAnimationState, StartContext, StopMode, TickContext,
};
use crate::handle::{AnimationHandle, HandleInner, PreObservers};
use crate::spec::{AnimationOptions, FolderFlagOp, IconAnimationSpec};
use crate::{DesktopError, IconId, IconSnapshot, Point, TimelineCloseMode};

pub(crate) enum StartMode {
    Animation,
    Cancel,
    Timeline { runtime: crate::timeline::Runtime, ready: Sender<()> },
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default tick period — 10 ms (100 Hz), matching the legacy C
/// `POS_CHANGE_PER_N = 10_000 µs` exactly.
///
/// This rate is only smooth **because the Windows shell backend pumps
/// its worker thread's message queue after every commit** (see
/// `pump_thread_messages` in `crates/rdi-platform-windows/src/backend.rs`
/// for the full explanation). Without the pump, cross-STA `SendMessage`
/// traffic from Explorer back into the worker piles up and Explorer's
/// UI thread stalls until the animation ends.
///
/// Callers can override via [`AnimationOptions::tick_hz`].
const DEFAULT_TICK: StdDuration = StdDuration::from_millis(10);

/// Default position-error tolerance in pixels — matches legacy
/// `POSITION_ERROR_VALUE = 10`.
const DEFAULT_TOLERANCE_PX: i32 = 10;

// ---------------------------------------------------------------------------
// Worker messages
// ---------------------------------------------------------------------------

pub(crate) enum WorkerMsg {
    PrepareScene {
        scene: crate::Scene,
        resp: Sender<Result<crate::RenderSession, DesktopError>>,
    },
    // --- Sync request/reply commands.
    ListIcons {
        resp: Sender<Result<Vec<IconSnapshot>, DesktopError>>,
    },
    GetFlags {
        resp: Sender<Result<u32, DesktopError>>,
    },
    ApplyFlags {
        mask: u32,
        values: u32,
        resp: Sender<Result<(), DesktopError>>,
    },
    SetPositions {
        moves: Vec<(IconId, Point)>,
        resp: Sender<Result<Vec<IconId>, DesktopError>>,
    },
    ListMonitors {
        resp: Sender<Result<Vec<crate::MonitorInfo>, DesktopError>>,
    },
    DesktopInfo {
        resp: Sender<Result<crate::DesktopInfo, DesktopError>>,
    },
    RenderOverlaySnapshot {
        width_px: u32,
        height_px: u32,
        dpi_scale: f32,
        positions: Vec<(IconId, Point)>,
        render_options: crate::OverlayRenderOptions,
        resp: Sender<Result<crate::SnapshotFrame, DesktopError>>,
    },

    // --- Animation lifecycle.
    StartAnimation {
        specs: Vec<IconAnimationSpec>,
        options: AnimationOptions,
        observers: PreObservers,
        resp: Sender<Result<AnimationHandle, DesktopError>>,
        preparation: Option<(Sender<()>, Receiver<StartMode>)>,
    },

    // --- In-flight animation control (received between ticks).
    AnimStop {
        mode: StopMode,
        owner: std::sync::Weak<HandleInner>,
    },

    Shutdown,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub(crate) fn worker_main(
    tx: Sender<WorkerMsg>,
    rx: Receiver<WorkerMsg>,
    mut backend: Box<dyn DesktopBackend>,
) {
    // `tx` is held for the thread's lifetime so a clone can be handed
    // to each `AnimationHandle`. Without it the channel would appear
    // disconnected the moment the controller drops its sender.
    while let Ok(msg) = rx.recv() {
        match msg {
            WorkerMsg::Shutdown => break,
            WorkerMsg::PrepareScene { scene, resp } => {
                let prepared = (|| {
                    let scene = crate::scene::EvaluatedScene::new(scene)?;
                    let icons = backend.list_icons()?;
                    if scene.scene.icons.iter().any(|entry| !icons.iter().any(|icon| icon.id == entry.animation.id)) {
                        return Err(DesktopError::BackendUnavailable("scene contains an unavailable desktop icon".into()));
                    }
                    let renderer = backend.prepare_scene_renderer(scene.scene.canvas, &scene.plans(), scene.scene.render_options)?;
                    Ok((scene, renderer))
                })();
                match prepared {
                    Err(error) => { let _ = resp.send(Err(error)); }
                    Ok((scene, mut renderer)) => {
                        let (requests, frames) = crossbeam_channel::unbounded();
                        let session = crate::RenderSession { tx: requests, worker: std::thread::current().id(), duration: scene.duration };
                        if resp.send(Ok(session)).is_err() { continue; }
                        let handle = HandleInner::new(tx.clone());
                        loop {
                            crossbeam_channel::select! {
                                recv(frames) -> request => match request {
                                    Ok(crate::scene::RenderRequest::Frame(seconds, reply)) => {
                                        let result = scene.sample(seconds).and_then(|frame| renderer.render(&frame, seconds));
                                        let _ = reply.send(result);
                                    }
                                    Ok(crate::scene::RenderRequest::Close(reply)) => {
                                        drop(renderer);
                                        let _ = reply.send(());
                                        break;
                                    }
                                    Err(_) => break,
                                },
                                recv(rx) -> message => match message {
                                    Ok(message) => match handle_control(message, &mut Vec::new(), &handle, 0) {
                                        Control::ControlSync(command) => run_sync_command(command, &mut *backend),
                                        Control::Shutdown => return,
                                        _ => {}
                                    },
                                    Err(_) => return,
                                }
                            }
                        }
                    }
                }
            }
            WorkerMsg::ListIcons { resp } => {
                let _ = resp.send(backend.list_icons());
            }
            WorkerMsg::GetFlags { resp } => {
                let _ = resp.send(backend.get_flags());
            }
            WorkerMsg::ApplyFlags { mask, values, resp } => {
                let _ = resp.send(backend.apply_flags(mask, values));
            }
            WorkerMsg::SetPositions { moves, resp } => {
                let _ = resp.send(backend.set_positions(&moves));
            }
            WorkerMsg::ListMonitors { resp } => {
                let _ = resp.send(backend.list_monitors());
            }
            WorkerMsg::DesktopInfo { resp } => {
                run_sync_command(SyncCommand::DesktopInfo(resp), &mut *backend);
            }
            WorkerMsg::RenderOverlaySnapshot {
                width_px,
                height_px,
                dpi_scale,
                positions,
                render_options,
                resp,
            } => {
                let _ = resp.send(backend.render_overlay_snapshot(
                    width_px,
                    height_px,
                    dpi_scale,
                    &positions,
                    render_options,
                ));
            }
            WorkerMsg::StartAnimation {
                specs,
                options,
                observers,
                resp,
                preparation,
            } => {
                let handle_inner = HandleInner::new(tx.clone());
                // Install pre-observers atomically *before* handing the
                // handle back to the caller so the worker can fire events
                // through them from the very first tick.
                handle_inner.install_pre_observers(observers);
                let handle = AnimationHandle::from_inner(handle_inner.clone());
                let _ = resp.send(Ok(handle));
                if let WorkerFate::Shutdown =
                    run_animation(&mut *backend, &rx, specs, options, handle_inner, preparation)
                {
                    break;
                }
            }
            // Stray stop arriving while no animation is running: ignore.
            WorkerMsg::AnimStop { .. } => {}
        }
    }
}

/// Whether the worker should keep running after an animation ends or shut
/// down entirely (because a `Shutdown` message was seen mid-animation).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum WorkerFate {
    Continue,
    Shutdown,
}

// ---------------------------------------------------------------------------
// Per-icon animation state (worker-owned)
// ---------------------------------------------------------------------------

struct AnimState {
    id: IconId,
    origin: Point,
    current: Point,
    target: Point,
    duration: StdDuration,
    start_at: Instant,
    curve_x: crate::curve::Curve,
    curve_y: crate::curve::Curve,
    effect: Option<crate::Effect>,
    is_final: bool,
}

impl AnimState {
    fn position_at(&self, progress: f32) -> Point {
        crate::scene::position_at(self.origin, self.target, &self.curve_x, &self.curve_y, progress)
    }

    fn timeline_t(&self, position: f64, elapsed: f64) -> f32 {
        crate::scene::progress_at(self.duration.as_secs_f64(), position, elapsed)
    }

    fn t_at(&self, now: Instant) -> f32 {
        let dur_s = self.duration.as_secs_f32();
        if dur_s <= 0.0 {
            return 1.0;
        }
        let elapsed = now.saturating_duration_since(self.start_at).as_secs_f32();
        (elapsed / dur_s).clamp(0.0, 1.0)
    }

    fn snapshot(&self, now: Instant) -> IconAnimationState {
        IconAnimationState {
            id: self.id.clone(),
            origin: self.origin,
            current: self.current,
            target: self.target,
            t: if self.is_final { 1.0 } else { self.t_at(now) },
            is_final: self.is_final,
        }
    }
}

// ---------------------------------------------------------------------------
// Animation execution
// ---------------------------------------------------------------------------

fn run_animation(
    backend: &mut dyn DesktopBackend,
    rx: &Receiver<WorkerMsg>,
    mut specs: Vec<IconAnimationSpec>,
    options: AnimationOptions,
    handle: Arc<HandleInner>,
    preparation: Option<(Sender<()>, Receiver<StartMode>)>,
) -> WorkerFate {
    let mut fate = WorkerFate::Continue;
    let mut timeline = None;
    let mut timeline_ready = None;
    let span = info_span!("animation", icons = specs.len());
    let _guard = span.enter();

    // --- 1. Snapshot the desktop
    let icons = match backend.list_icons() {
        Ok(v) => v,
        Err(e) => {
            finalize_with_error(&handle, &e);
            return fate;
        }
    };
    if options.snap_to_grid && specs.iter().any(|spec| icons.iter().any(|icon| icon.id == spec.id)) {
        let resolution = backend.desktop_info(&icons).and_then(|desktop| {
            crate::resolve_grid_targets(&mut specs, &icons, &desktop)
        });
        if let Err(error) = resolution {
            finalize_with_error(&handle, &error);
            return fate;
        }
    }
    let mut by_id: HashMap<IconId, Point> =
        icons.into_iter().map(|s| (s.id, s.position)).collect();

    // --- 2. Build the active set + missing list
    let started_at = Instant::now();
    let mut active: Vec<AnimState> = Vec::new();
    let mut missing: Vec<IconId> = Vec::new();
    for spec in specs {
        if let Some(effect) = &spec.effect {
            if let Err(error) = effect.validate() {
                finalize_with_error(&handle, &error);
                return fate;
            }
        }
        match by_id.remove(&spec.id) {
            Some(origin) => match spec.duration.resolve(origin, spec.target) {
                Ok(dur) => active.push(AnimState {
                    id: spec.id,
                    origin,
                    current: origin,
                    target: spec.target,
                    duration: dur,
                    start_at: started_at,
                    curve_x: spec.curve_x,
                    curve_y: spec.curve_y,
                    effect: spec.effect,
                    is_final: false,
                }),
                Err(e) => {
                    finalize_with_error(&handle, &e);
                    return fate;
                }
            },
            None => missing.push(spec.id),
        }
    }

    let total_icons = active.len();
    let missing_count = missing.len();
    info!(
        animating = total_icons,
        missing = missing_count,
        "animation starting"
    );

    // --- 3. Publish initial state and fire on_start
    {
        let mut state = handle.lock_state();
        state.running = true;
        state.progress = 0.0;
        state.missing = missing;
        state.per_icon = active.iter().map(|a| a.snapshot(started_at)).collect();
        state.finish_reason = None;
    }
    // --- 5. Try to open an overlay session
    let mut plans: Vec<crate::IconRenderPlan> = active
        .iter()
        .map(|entry| {
            let mut plan = crate::IconRenderPlan::placeholder(entry.id.clone(), entry.origin, entry.target);
            plan.effect = entry.effect.clone();
            plan
        })
        .collect();

    let mut stationary: Vec<_> = by_id.into_iter().collect();
    stationary.sort_by(|left, right| left.0.as_str().cmp(right.0.as_str()));
    plans.extend(stationary.into_iter().map(|(id, position)| {
        let mut plan = crate::IconRenderPlan::placeholder(id, position, position);
        plan.stationary = true;
        plan
    }));

    let requires_overlay = preparation.is_some() || active.iter().any(|entry| entry.effect.is_some());
    if options.force_fallback && requires_overlay {
        finalize_with_error(&handle, &DesktopError::InvalidEffect("preparation and effects require an overlay".into()));
        return fate;
    }

    let overlay_open = if options.force_fallback {
        eprint_fallback_banner(
            "AnimationOptions::force_fallback is true — running without overlay",
        );
        false
    } else {
        match backend.begin_overlay_session(&plans, options.render_options) {
            Ok(()) => {
                debug!(plans = plans.len(), "overlay session opened");
                true
            }
            Err(DesktopError::OverlayUnavailable(reason)) if !requires_overlay => {
                eprint_fallback_banner(&format!(
                    "backend reported OverlayUnavailable: {reason}"
                ));
                false
            }
            Err(e) => {
                finalize_with_error(&handle, &e);
                return fate;
            }
        }
    };

    if let Some((ready, start)) = preparation {
        let _ = ready.send(());
        loop {
            crossbeam_channel::select! {
                recv(start) -> requested => {
                    match requested {
                        Ok(StartMode::Animation) => break,
                        Ok(StartMode::Timeline { runtime, ready }) => {
                            timeline = Some(runtime);
                            timeline_ready = Some(ready);
                            break;
                        }
                        _ => {}
                    }
                    backend.discard_overlay_session();
                    publish_finish_state(&handle, &active, FinishReason::Stopped(StopMode::LeaveInPlace), None, None);
                    return fate;
                }
                recv(rx) -> message => {
                    let control = message.map(|msg| handle_control(msg, &mut active, &handle, 0)).unwrap_or(Control::Shutdown);
                    match control {
                        Control::Continue => {},
                        Control::ControlSync(command) => run_sync_command(command, backend),
                        other => {
                            backend.discard_overlay_session();
                            publish_finish_state(&handle, &active, FinishReason::Stopped(StopMode::LeaveInPlace), None, None);
                            return if matches!(other, Control::Shutdown) { WorkerFate::Shutdown } else { fate };
                        }
                    }
                }
                default(StdDuration::from_millis(50)) => {
                    if let Err(error) = backend.validate_prepared_session() {
                        backend.discard_overlay_session();
                        finalize_with_error(&handle, &error);
                        return fate;
                    }
                }
            }
        }
    }

    if overlay_open {
        if let Err(error) = backend.validate_prepared_session() {
            backend.discard_overlay_session();
            finalize_with_error(&handle, &error);
            return fate;
        }
    }

    if let Some(op) = options.before_flags {
        let _ = apply_flag_op(backend, op);
    }

    if overlay_open {
        let initial: Vec<_> = active.iter().map(|entry| crate::IconFrame {
            id: entry.id.clone(), position: entry.origin, progress: 0.0, elapsed_seconds: 0.0,
        }).collect();
        if let Err(error) = backend.commit_visual_frame(&initial) {
            let cancelled = matches!(error, DesktopError::OverlayCancelled(_));
            let positions: Vec<_> = active.iter_mut().map(|entry| {
                if cancelled { entry.current = entry.target; entry.is_final = true; }
                (entry.id.clone(), entry.current)
            }).collect();
            let outcome = backend.finalize_overlay_session(&positions).ok();
            if let Some(op) = options.after_flags { let _ = apply_flag_op(backend, op); }
            let reason = if cancelled { FinishReason::Stopped(StopMode::TeleportToTarget) }
                else { FinishReason::Error(error.to_string()) };
            publish_finish_state(&handle, &active, reason, outcome, timeline.as_mut());
            return fate;
        }
    }
    let started_at = Instant::now();
    for entry in &mut active { entry.start_at = started_at; }
    if let Some(runtime) = &mut timeline {
        runtime.activate(active.iter().map(|entry| entry.duration.as_secs_f64()).fold(0.0, f64::max));
        runtime.publish(0.0, active.iter().map(|entry| {
            let mut snapshot = entry.snapshot(started_at);
            snapshot.t = 0.0;
            snapshot
        }).collect());
        if let Some(ready) = timeline_ready.take() { let _ = ready.send(()); }
    }
    for callback in &handle.observers_snapshot().on_start {
        callback(&StartContext { total_icons, missing_icons: missing_count, started_at });
    }

    // --- 6. Fallback path: no overlay, direct teleport
    
    if !overlay_open {
        let targets: Vec<(IconId, Point)> = active
            .iter()
            .map(|a| (a.id.clone(), a.target))
            .collect();

        // Apply targets and update the in-memory state so the finish
        // observers see the right per-icon state.
        let commit_result = backend.set_positions(&targets);

        if let Some(op) = options.after_flags {
            let _ = apply_flag_op(backend, op);
        }

        // Mark every remaining icon final.
        let vanished: Vec<IconId> = match commit_result {
            Ok(missing_now) => missing_now,
            Err(e) => {
                finalize_with_error(&handle, &e);
                return fate;
            }
        };
        if !vanished.is_empty() {
            warn!(
                unresolved = vanished.len(),
                "fallback commit could not resolve every icon"
            );
        }
        for a in active.iter_mut() {
            a.current = a.target;
            a.is_final = true;
        }

        // `moved_ids` stays empty: the fallback path issues no
        // confirmation poll, so nothing was confirmed by the Shell.
        let outcome = crate::FinalCommitOutcome {
            moved_ids: Vec::new(),
            missing_ids: vanished,
        };
        publish_finish_state(&handle, &active, FinishReason::Completed, Some(outcome), None);
        return fate;
    }

    // --- 7. Overlay tick loop.
    let period = options
        .tick_hz
        .map(|hz| StdDuration::from_secs_f64(1.0 / hz.max(1) as f64))
        .unwrap_or(DEFAULT_TICK);
    let tolerance = options.position_tolerance_px.unwrap_or(DEFAULT_TOLERANCE_PX);
    let tolerance_abs = tolerance.abs();

    let mut finish = FinishReason::Completed;
    let mut next_tick = Instant::now() + period;
    let mut finalized_count: usize = 0;
    let timeline_rx = timeline.as_ref().map(|runtime| runtime.rx.clone()).unwrap_or_else(crossbeam_channel::never);

    'outer: loop {
        let mut dirty = timeline.as_ref().is_none_or(|runtime| runtime.playing());
        // --- 7a. Drain queued control messages until the next tick.
        loop {
            let now = Instant::now();
            if now >= next_tick {
                break;
            }
            let message = crossbeam_channel::select! {
                recv(timeline_rx) -> request => {
                    if let Some(runtime) = &mut timeline {
                        let close = match request {
                            Ok(request) => runtime.control(request, Instant::now(), backend),
                            Err(_) => Some(TimelineCloseMode::RestoreOrigins),
                        };
                        if let Some(mode) = close {
                            for entry in &mut active {
                                entry.current = match mode {
                                    TimelineCloseMode::RestoreOrigins => entry.origin,
                                    TimelineCloseMode::LeaveInPlace => entry.current,
                                    TimelineCloseMode::TeleportToTarget => entry.target,
                                };
                            }
                            finish = FinishReason::Stopped(if mode == TimelineCloseMode::TeleportToTarget {
                                StopMode::TeleportToTarget
                            } else { StopMode::LeaveInPlace });
                            runtime.acknowledge();
                            break 'outer;
                        }
                        dirty = true;
                    }
                    break;
                }
                recv(rx) -> message => message.map_err(|_| RecvTimeoutError::Disconnected),
                default(next_tick.saturating_duration_since(now)) => Err(RecvTimeoutError::Timeout),
            };
            match message {
                Ok(msg) => match handle_control(msg, &mut active, &handle, tolerance_abs) {
                    Control::Continue => {}
                    Control::Stop(reason) => {
                        finish = reason;
                        break 'outer;
                    }
                    Control::Shutdown => {
                        if timeline.is_some() {
                            for entry in &mut active { entry.current = entry.origin; }
                        }
                        finish = FinishReason::Stopped(StopMode::LeaveInPlace);
                        fate = WorkerFate::Shutdown;
                        break 'outer;
                    }
                    Control::ControlSync(sync) => {
                        // Read-only commands are serviced here so they
                        // stay serial with respect to the tick loop.
                        // Writes are rejected in `handle_control` — see
                        // the `AnimationBusy` arms there.
                        run_sync_command(sync, backend);
                    }
                },
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    // All external senders and even the worker's own `tx`
                    // must be gone for this to happen — treat as shutdown.
                    finish = FinishReason::Stopped(StopMode::LeaveInPlace);
                    fate = WorkerFate::Shutdown;
                    break 'outer;
                }
            }
        }

        if active.is_empty() && timeline.is_none() {
            break 'outer;
        }

        // NOTE: `set_positions` is deliberately not called here. The
        // overlay is the source of visible truth for the entire
        // animation; `finalize_overlay_session` is what eventually
        // hands the final positions to Explorer.
        let now = Instant::now();
        let timeline_sample = timeline.as_mut().map(|runtime| runtime.sample(now));
        let mut newly_final: Vec<IconId> = Vec::new();
        let mut frame: Vec<(IconId, Point)> = Vec::with_capacity(active.len());

        for a in active.iter_mut() {
            if let Some((position, elapsed)) = timeline_sample {
                let progress = a.timeline_t(position, elapsed);
                a.current = if progress <= 0.0 { a.origin } else if progress >= 1.0 { a.target }
                    else { a.position_at(progress) };
                frame.push((a.id.clone(), a.current));
                continue;
            }
            if a.is_final {
                // Keep already-final icons at their target for the
                // remainder of the animation so overlay copies don't
                // vanish visually.
                frame.push((a.id.clone(), a.target));
                continue;
            }
            let t = a.t_at(now);
            let new_pos = a.position_at(t);

            let close_enough = (new_pos.x - a.target.x).abs() < tolerance_abs
                && (new_pos.y - a.target.y).abs() < tolerance_abs;

            if t >= 1.0 || (close_enough && a.effect.is_none()) {
                a.current = a.target;
                a.is_final = true;
                newly_final.push(a.id.clone());
                frame.push((a.id.clone(), a.target));
            } else {
                a.current = new_pos;
                frame.push((a.id.clone(), new_pos));
            }
        }

        if !frame.is_empty() || timeline.is_some() {
            let visual_frame: Vec<_> = active.iter().map(|entry| crate::IconFrame {
                id: entry.id.clone(), position: entry.current,
                progress: timeline_sample.map_or_else(
                    || if entry.is_final { 1.0 } else { entry.t_at(now) },
                    |(position, elapsed)| entry.timeline_t(position, elapsed)),
                elapsed_seconds: timeline_sample.map_or_else(
                    || now.saturating_duration_since(started_at).as_secs_f32(),
                    |(_, elapsed)| elapsed as f32),
            }).collect();
            let submitted = if dirty { backend.commit_visual_frame(&visual_frame) }
                else { backend.poll_overlay_session() };
            match submitted {
                Ok(()) => {}
                Err(DesktopError::OverlayCancelled(reason)) => {
                    // Graceful cancellation — the backend detected a
                    // shell disruption (Explorer restarted / display
                    // topology changed / DPI changed / shell view
                    // lost). Because the first-frame teleport
                    // already moved the real icons to their target
                    // positions, snapping in-memory state to targets
                    // and finishing with `Stopped(TeleportToTarget)`
                    // accurately reflects the visible outcome. The
                    // engine still runs `finalize_overlay_session`
                    // below to unhide the real icons and destroy
                    // the overlay window.
                    warn!(%reason, "overlay animation cancelled");
                    for a in active.iter_mut() {
                        if !a.is_final {
                            a.current = a.target;
                            a.is_final = true;
                        }
                    }
                    finish = FinishReason::Stopped(StopMode::TeleportToTarget);
                    break 'outer;
                }
                Err(e) => {
                    finish = FinishReason::Error(format!(
                        "commit_overlay_frame failed: {e}"
                    ));
                    break 'outer;
                }
            }
            if let (Some(runtime), Some((position, elapsed))) = (&mut timeline, timeline_sample) {
                runtime.capture_pending(backend, elapsed);
                runtime.publish(position, active.iter().zip(&visual_frame).map(|(entry, frame)| {
                    let mut snapshot = entry.snapshot(now);
                    snapshot.t = frame.progress;
                    snapshot
                }).collect());
            }
        }

        if let Some(runtime) = &timeline {
            next_tick = Instant::now() + if runtime.playing() { period } else { StdDuration::from_millis(50) };
            continue;
        }

        finalized_count += newly_final.len();

        // --- 7c. Publish snapshot and fire observers.
        let progress = mean_progress(&active, now);
        trace!(progress, newly_final = newly_final.len(), "tick");
        {
            let mut state = handle.lock_state();
            state.progress = progress;
            state.per_icon = active.iter().map(|a| a.snapshot(now)).collect();
        }

        let observers = handle.observers_snapshot();
        for id in &newly_final {
            for cb in &observers.on_icon_complete {
                cb(id);
            }
        }
        let elapsed = now.saturating_duration_since(started_at);
        let active_icons = active.iter().filter(|a| !a.is_final).count();
        for cb in &observers.on_tick {
            cb(&TickContext {
                elapsed,
                progress,
                active_icons,
                finalized_icons: finalized_count,
            });
        }

        // --- 7d. Termination check.
        if active.iter().all(|a| a.is_final) {
            break 'outer;
        }

        next_tick += period;
        if next_tick < Instant::now() { next_tick = Instant::now(); }
    }

    // --- 8. Finalize the overlay session with the true final positions
    let final_positions: Vec<(IconId, Point)> = active
        .iter()
        .map(|a| (a.id.clone(), a.current))
        .collect();

    let final_commit = match backend.finalize_overlay_session(&final_positions) {
        Ok(outcome) => {
            if !outcome.missing_ids.is_empty() {
                // The backend could not resolve these ids at commit
                // time, so they are NOT at their targets.
                warn!(
                    unresolved = outcome.missing_ids.len(),
                    "final commit could not resolve every icon; they were left \
                     wherever the Shell last had them"
                );
            }
            Some(outcome)
        }
        Err(e) => {
            // Finalization failure is bad — the overlay may still be up
            // and the real icons hidden. Surface it as an Error finish
            // reason, but still attempt to restore the flags below.
            if timeline.is_some() || matches!(finish, FinishReason::Completed) {
                finish = FinishReason::Error(format!(
                    "finalize_overlay_session failed: {e}"
                ));
            } else {
                warn!(
                    error = %e,
                    ?finish,
                    "finalize_overlay_session failed on top of an existing failure"
                );
            }
            None
        }
    };

    // --- 9. Apply after-flags
    if let Some(op) = options.after_flags {
        let _ = apply_flag_op(backend, op);
    }

    // --- 10. Publish finish state and notify waiters
    info!(?finish, ticks_finalized = finalized_count, "animation finished");
    publish_finish_state(&handle, &active, finish, final_commit, timeline.as_mut());

    fate
}

/// Publish the terminal state of an animation onto its handle and fire
/// the `on_finish` observers.
///
/// Ordering matters: (1) record `finish_reason` while `running ==
/// true` so callbacks that inspect the handle see the outcome, (2) fire
/// `on_finish`, and (3) *only then* flip `running` and notify waiters.
/// Doing (3) before (2) would let a caller blocked in `handle.wait()`
/// wake up before the observer callbacks have fired — see the
/// `observers_fire_in_expected_order` integration test.
fn publish_finish_state(
    handle: &Arc<HandleInner>,
    active: &[AnimState],
    finish: FinishReason,
    final_commit: Option<crate::FinalCommitOutcome>,
    timeline: Option<&mut crate::timeline::Runtime>,
) {
    {
        let mut state = handle.lock_state();
        state.progress = 1.0;
        state.finish_reason = Some(finish.clone());
        state.final_commit = final_commit;
        for entry in state.per_icon.iter_mut() {
            for a in active {
                if a.id == entry.id {
                    entry.current = a.current;
                    entry.is_final = a.is_final;
                    entry.t = if a.is_final { 1.0 } else { entry.t };
                    break;
                }
            }
        }
    }

    if let Some(runtime) = timeline { runtime.finish(); }

    let observers = handle.observers_snapshot();
    for cb in &observers.on_finish {
        cb(&finish);
    }

    {
        let mut state = handle.lock_state();
        state.running = false;
    }
    handle.finish_cv.notify_all();
}

/// Report why the engine is taking the fallback path.
///
/// `warn!` rather than `error!` because the animation still completes —
/// the icons just teleport instead of gliding.
fn eprint_fallback_banner(reason: &str) {
    warn!(
        reason,
        "overlay renderer unavailable — icons will teleport to their targets \
         without animation"
    );
}

// ---------------------------------------------------------------------------
// Control-message dispatch
// ---------------------------------------------------------------------------

enum Control {
    Continue,
    Stop(FinishReason),
    Shutdown,
    /// A **read-only** sync request (list_icons, get_flags,
    /// list_monitors, render_overlay_snapshot) arrived mid-animation.
    /// The engine services it out of band via [`run_sync_command`] so
    /// it stays serial with respect to the tick loop. Mutating
    /// requests never reach this variant.
    ControlSync(SyncCommand),
}

enum SyncCommand {
    ListIcons(Sender<Result<Vec<IconSnapshot>, DesktopError>>),
    GetFlags(Sender<Result<u32, DesktopError>>),
    ListMonitors(Sender<Result<Vec<crate::MonitorInfo>, DesktopError>>),
    DesktopInfo(Sender<Result<crate::DesktopInfo, DesktopError>>),
    RenderOverlaySnapshot {
        width_px: u32,
        height_px: u32,
        dpi_scale: f32,
        positions: Vec<(IconId, Point)>,
        render_options: crate::OverlayRenderOptions,
        resp: Sender<Result<crate::SnapshotFrame, DesktopError>>,
    },
}

fn run_sync_command(cmd: SyncCommand, backend: &mut dyn DesktopBackend) {
    match cmd {
        SyncCommand::ListIcons(resp) => {
            let _ = resp.send(backend.list_icons());
        }
        SyncCommand::GetFlags(resp) => {
            let _ = resp.send(backend.get_flags());
        }
        SyncCommand::ListMonitors(resp) => {
            let _ = resp.send(backend.list_monitors());
        }
        SyncCommand::DesktopInfo(resp) => {
            let result = backend.list_icons().and_then(|icons| backend.desktop_info(&icons));
            let _ = resp.send(result);
        }
        SyncCommand::RenderOverlaySnapshot {
            width_px,
            height_px,
            dpi_scale,
            positions,
            render_options,
            resp,
        } => {
            let _ = resp.send(backend.render_overlay_snapshot(
                width_px,
                height_px,
                dpi_scale,
                &positions,
                render_options,
            ));
        }
    }
}

fn handle_control(
    msg: WorkerMsg,
    active: &mut Vec<AnimState>,
    handle: &Arc<HandleInner>,
    tolerance_abs: i32,
) -> Control {
    // `tolerance_abs` is currently unused inside message handling but is
    // reserved for future control messages (e.g. re-tune tolerance mid-flight).
    let _ = tolerance_abs;

    match msg {
        WorkerMsg::PrepareScene { resp, .. } => {
            let _ = resp.send(Err(DesktopError::AnimationBusy));
            Control::Continue
        }
        WorkerMsg::AnimStop { mode, owner } => {
            if !owner.ptr_eq(&Arc::downgrade(handle)) { return Control::Continue; }
            if matches!(mode, StopMode::TeleportToTarget) {
                // Snap every remaining icon to its target in memory —
                // the overlay will pick this up on the next commit and
                // `finalize_overlay_session` will push it to the Shell.
                for a in active.iter_mut() {
                    if !a.is_final {
                        a.current = a.target;
                        a.is_final = true;
                    }
                }
            }
            Control::Stop(FinishReason::Stopped(mode))
        }
        WorkerMsg::Shutdown => Control::Shutdown,

        // Read-only commands arrive during an animation too — hand
        // them over via the ControlSync variant so the tick loop can
        // service them without duplicating handler logic.
        WorkerMsg::ListIcons { resp } => Control::ControlSync(SyncCommand::ListIcons(resp)),
        WorkerMsg::GetFlags { resp } => Control::ControlSync(SyncCommand::GetFlags(resp)),
        WorkerMsg::ListMonitors { resp } => {
            Control::ControlSync(SyncCommand::ListMonitors(resp))
        }
        WorkerMsg::DesktopInfo { resp } => Control::ControlSync(SyncCommand::DesktopInfo(resp)),

        // Shell writes are refused for the duration of the animation.
        // A mid-flight `apply_flags` can clear `FWF_HIDEICONS` and
        // un-hide the real icons behind the overlay; a mid-flight
        // `set_positions` moves icons the final commit then overwrites.
        // Callers bracket animations with `AnimationOptions::
        // before_flags` / `after_flags` instead.
        WorkerMsg::ApplyFlags { resp, .. } => {
            let _ = resp.send(Err(DesktopError::AnimationBusy));
            Control::Continue
        }
        WorkerMsg::SetPositions { resp, .. } => {
            let _ = resp.send(Err(DesktopError::AnimationBusy));
            Control::Continue
        }
        WorkerMsg::RenderOverlaySnapshot {
            width_px,
            height_px,
            dpi_scale,
            positions,
            render_options,
            resp,
        } => Control::ControlSync(SyncCommand::RenderOverlaySnapshot {
            width_px,
            height_px,
            dpi_scale,
            positions,
            render_options,
            resp,
        }),
        WorkerMsg::StartAnimation { resp, .. } => {
            let _ = resp.send(Err(DesktopError::AnimationBusy));
            Control::Continue
        }
    }
}

fn apply_flag_op(
    backend: &mut dyn DesktopBackend,
    op: FolderFlagOp,
) -> Result<(), DesktopError> {
    match op {
        FolderFlagOp::Set(flags) => backend.apply_flags(flags, 0xFFFF_FFFF),
        FolderFlagOp::Exactly(flags) => backend.apply_flags(build_true_mask(flags), flags),
    }
}

/// Reproduce the legacy `buildTrueMask` helper: given a set of bits, return
/// the smallest `2^n − 1` mask that covers them.
///
/// Used by the `Exactly` variant of [`FolderFlagOp`].
pub(crate) fn build_true_mask(flags: u32) -> u32 {
    if flags == 0 {
        return 0;
    }
    let highest = 31 - flags.leading_zeros();
    if highest == 31 {
        u32::MAX
    } else {
        (1u32 << (highest + 1)) - 1
    }
}

fn finalize_with_error(handle: &Arc<HandleInner>, err: &DesktopError) {
    let reason = FinishReason::Error(err.to_string());
    // Same ordering as the normal termination path: publish
    // `finish_reason`, fire `on_finish`, *then* flip `running` and
    // notify waiters. See the comment in `run_animation`'s section 7.
    {
        let mut state = handle.lock_state();
        state.finish_reason = Some(reason.clone());
    }
    let observers = handle.observers_snapshot();
    for cb in &observers.on_finish {
        cb(&reason);
    }
    {
        let mut state = handle.lock_state();
        state.running = false;
    }
    handle.finish_cv.notify_all();
}

fn mean_progress(active: &[AnimState], now: Instant) -> f32 {
    if active.is_empty() {
        return 1.0;
    }
    let sum: f32 = active
        .iter()
        .map(|a| if a.is_final { 1.0 } else { a.t_at(now) })
        .sum();
    sum / active.len() as f32
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_true_mask_matches_legacy() {
        assert_eq!(build_true_mask(0), 0);
        assert_eq!(build_true_mask(0b0001), 0b0001);
        assert_eq!(build_true_mask(0b0100), 0b0111);
        assert_eq!(build_true_mask(0b1010_0000), 0b1111_1111);
        assert_eq!(build_true_mask(1 << 31), u32::MAX);
    }

    #[test]
    fn mean_progress_empty_is_one() {
        assert_eq!(mean_progress(&[], Instant::now()), 1.0);
    }
}

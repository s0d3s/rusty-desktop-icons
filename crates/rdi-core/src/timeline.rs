use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};

use crate::{AnimationHandle, DesktopError, FinalCommitOutcome, FinishReason, IconAnimationState, IconId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineCloseMode {
    RestoreOrigins,
    LeaveInPlace,
    TeleportToTarget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineState {
    Paused,
    Playing,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackOutcome {
    Reached,
    Interrupted,
    Closed,
}

struct State {
    position: f64,
    speed: f64,
    phase: TimelineState,
    worker: Option<ThreadId>,
    icons: Vec<IconAnimationState>,
    real_icons_visible: bool,
}

type Shared = Arc<Mutex<State>>;

fn lock(shared: &Shared) -> MutexGuard<'_, State> {
    shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn check_thread(shared: &Shared) -> Result<(), DesktopError> {
    if lock(shared).worker == Some(std::thread::current().id()) {
        return Err(DesktopError::BackendUnavailable("blocking timeline calls cannot run on the animation worker".into()));
    }
    Ok(())
}

fn closed_error() -> DesktopError {
    DesktopError::BackendUnavailable("timeline session is closed".into())
}

pub struct PlaybackHandle {
    result: Arc<Completion>,
    shared: Shared,
}

#[derive(Default)]
struct Completion {
    outcome: Mutex<Option<PlaybackOutcome>>,
    ready: Condvar,
}

impl Completion {
    fn complete(&self, outcome: PlaybackOutcome) {
        *self.outcome.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
        self.ready.notify_all();
    }
}

impl PlaybackHandle {
    pub fn wait(&self) -> Result<PlaybackOutcome, DesktopError> {
        check_thread(&self.shared)?;
        let state = self.result.outcome.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = self.result.ready.wait_while(state, |outcome| outcome.is_none())
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(state.unwrap())
    }

    pub fn wait_timeout(&self, timeout: Duration) -> Result<Option<PlaybackOutcome>, DesktopError> {
        check_thread(&self.shared)?;
        let state = self.result.outcome.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (state, _) = self.result.ready.wait_timeout_while(state, timeout, |outcome| outcome.is_none())
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(*state)
    }
}

pub struct TimelineSession {
    tx: Sender<Request>,
    shared: Shared,
    handle: AnimationHandle,
}

impl TimelineSession {
    pub(crate) fn pair(handle: AnimationHandle) -> (Self, Runtime) {
        let (tx, rx) = unbounded();
        let shared = Arc::new(Mutex::new(State {
            position: 0.0, speed: 1.0, phase: TimelineState::Paused,
            worker: None, icons: Vec::new(), real_icons_visible: false,
        }));
        let runtime = Runtime {
            rx, shared: shared.clone(), clock: Clock::new(0.0, Instant::now()),
            playback: None, acknowledgements: Vec::new(), captures: Vec::new(), reached: false,
        };
        (Self { tx, shared, handle }, runtime)
    }

    fn request(&self, command: Command) -> Result<(), DesktopError> {
        check_thread(&self.shared)?;
        let (reply, response) = bounded(1);
        self.tx.send(Request { command, reply }).map_err(|_| closed_error())?;
        response.recv().map_err(|_| closed_error())?
    }

    pub fn play_to(&self, position: f64, speed: f64) -> Result<PlaybackHandle, DesktopError> {
        validate_position(position)?;
        validate_speed(speed)?;
        let result = Arc::new(Completion::default());
        self.request(Command::Play { position, speed, result: result.clone() })?;
        Ok(PlaybackHandle { result, shared: self.shared.clone() })
    }

    pub fn seek(&self, position: f64) -> Result<(), DesktopError> {
        validate_position(position)?;
        self.request(Command::Seek(position))
    }

    pub fn pause(&self) -> Result<(), DesktopError> { self.request(Command::Pause) }

    pub fn capture(&self) -> Result<crate::CapturedFrame, DesktopError> {
        let (reply, response) = bounded(1);
        self.request(Command::Capture(reply))?;
        response.recv().map_err(|_| closed_error())?
    }

    pub fn seek_and_capture(&self, position: f64) -> Result<crate::CapturedFrame, DesktopError> {
        validate_position(position)?;
        let (reply, response) = bounded(1);
        self.request(Command::SeekCapture(position, reply))?;
        response.recv().map_err(|_| closed_error())?
    }

    pub fn set_speed(&self, speed: f64) -> Result<(), DesktopError> {
        validate_speed(speed)?;
        self.request(Command::Speed(speed))
    }

    pub fn position(&self) -> f64 { lock(&self.shared).position }
    pub fn real_icons_visible(&self) -> bool { lock(&self.shared).real_icons_visible }

    pub fn set_real_icons_visible(&self, visible: bool) -> Result<(), DesktopError> {
        self.request(Command::RealIconsVisible(visible))
    }
    pub fn speed(&self) -> f64 { lock(&self.shared).speed }
    pub fn state(&self) -> TimelineState { lock(&self.shared).phase }
    pub fn snapshot(&self) -> Vec<IconAnimationState> { lock(&self.shared).icons.clone() }
    pub fn missing_icons(&self) -> Vec<IconId> { self.handle.missing_icons() }
    pub fn finish_reason(&self) -> Option<FinishReason> { self.handle.finish_reason() }
    pub fn final_commit(&self) -> Option<FinalCommitOutcome> { self.handle.final_commit() }

    pub fn close(&self, mode: TimelineCloseMode) -> Result<FinishReason, DesktopError> {
        check_thread(&self.shared)?;
        if self.state() != TimelineState::Closed {
            let _ = self.request(Command::Close(mode));
        }
        Ok(self.handle.wait())
    }
}

impl Drop for TimelineSession {
    fn drop(&mut self) {
        let (reply, _) = bounded(1);
        let _ = self.tx.send(Request { command: Command::Close(TimelineCloseMode::RestoreOrigins), reply });
    }
}

fn validate_position(position: f64) -> Result<(), DesktopError> {
    if !position.is_finite() || !(0.0..=1.0).contains(&position) {
        return Err(DesktopError::InvalidDuration("timeline position must be finite and within [0, 1]".into()));
    }
    Ok(())
}

fn validate_speed(speed: f64) -> Result<(), DesktopError> {
    if !speed.is_finite() || speed <= 0.0 {
        return Err(DesktopError::InvalidDuration("timeline speed must be finite and positive".into()));
    }
    Ok(())
}

pub(crate) struct Request {
    command: Command,
    reply: Sender<Result<(), DesktopError>>,
}

enum Command {
    Capture(Sender<Result<crate::CapturedFrame, DesktopError>>),
    SeekCapture(f64, Sender<Result<crate::CapturedFrame, DesktopError>>),
    Play { position: f64, speed: f64, result: Arc<Completion> },
    Seek(f64),
    Pause,
    Speed(f64),
    RealIconsVisible(bool),
    Close(TimelineCloseMode),
}

struct Clock {
    duration: f64,
    anchor: f64,
    wall: Instant,
    speed: f64,
    destination: Option<f64>,
}

impl Clock {
    fn new(duration: f64, now: Instant) -> Self {
        Self { duration, anchor: 0.0, wall: now, speed: 1.0, destination: None }
    }

    fn position_at(&self, now: Instant) -> f64 {
        let Some(destination) = self.destination else { return self.anchor; };
        if self.duration == 0.0 { return destination; }
        let distance = now.saturating_duration_since(self.wall).as_secs_f64() * self.speed / self.duration;
        if destination >= self.anchor { (self.anchor + distance).min(destination) }
        else { (self.anchor - distance).max(destination) }
    }

    fn reanchor(&mut self, now: Instant) {
        self.anchor = self.position_at(now);
        self.wall = now;
    }
}

pub(crate) struct Runtime {
    pub rx: Receiver<Request>,
    shared: Shared,
    clock: Clock,
    playback: Option<Arc<Completion>>,
    acknowledgements: Vec<Sender<Result<(), DesktopError>>>,
    captures: Vec<Sender<Result<crate::CapturedFrame, DesktopError>>>,
    reached: bool,
}

impl Runtime {
    pub fn activate(&mut self, duration: f64) {
        self.clock = Clock::new(duration, Instant::now());
        lock(&self.shared).worker = Some(std::thread::current().id());
    }

    fn end_playback(&mut self, outcome: PlaybackOutcome) {
        if let Some(reply) = self.playback.take() { reply.complete(outcome); }
    }

    pub fn control(&mut self, request: Request, now: Instant, backend: &mut dyn crate::DesktopBackend) -> Option<TimelineCloseMode> {
        if let Command::Capture(reply) = request.command {
            let seconds = lock(&self.shared).position * self.clock.duration;
            let _ = reply.send(backend.capture_overlay(seconds));
            let _ = request.reply.send(Ok(()));
            return None;
        }
        if let Command::RealIconsVisible(visible) = request.command {
            let result = backend.set_real_icons_visible(visible);
            if result.is_ok() { lock(&self.shared).real_icons_visible = visible; }
            let _ = request.reply.send(result);
            return None;
        }
        self.clock.reanchor(now);
        self.acknowledgements.push(request.reply);
        match request.command {
            Command::Capture(_) => unreachable!(),
            Command::SeekCapture(position, reply) => {
                self.end_playback(PlaybackOutcome::Interrupted);
                self.clock.anchor = position;
                self.clock.destination = None;
                self.captures.push(reply);
            }
            Command::Play { position, speed, result } => {
                self.end_playback(PlaybackOutcome::Interrupted);
                self.clock.destination = Some(position);
                self.clock.speed = speed;
                self.playback = Some(result);
            }
            Command::Seek(position) => {
                self.end_playback(PlaybackOutcome::Interrupted);
                self.clock.anchor = position;
                self.clock.destination = None;
            }
            Command::Pause => {
                self.end_playback(PlaybackOutcome::Interrupted);
                self.clock.destination = None;
            }
            Command::Speed(speed) => self.clock.speed = speed,
            Command::RealIconsVisible(_) => unreachable!(),
            Command::Close(mode) => return Some(mode),
        }
        None
    }

    pub fn sample(&mut self, now: Instant) -> (f64, f64) {
        let position = self.clock.position_at(now);
        self.reached = self.clock.destination == Some(position);
        if self.reached {
            self.clock.anchor = position;
            self.clock.destination = None;
        }
        (position, position * self.clock.duration)
    }

    pub fn playing(&self) -> bool { self.clock.destination.is_some() }

    pub fn capture_pending(&mut self, backend: &mut dyn crate::DesktopBackend, seconds: f64) {
        for reply in self.captures.drain(..) { let _ = reply.send(backend.capture_overlay(seconds)); }
    }

    pub fn publish(&mut self, position: f64, icons: Vec<IconAnimationState>) {
        {
            let mut state = lock(&self.shared);
            state.position = position;
            state.speed = self.clock.speed;
            state.phase = if self.playing() { TimelineState::Playing } else { TimelineState::Paused };
            state.icons = icons;
        }
        if self.reached { self.end_playback(PlaybackOutcome::Reached); }
        self.acknowledge();
    }

    pub fn acknowledge(&mut self) {
        for reply in self.acknowledgements.drain(..) { let _ = reply.send(Ok(())); }
    }

    pub fn finish(&mut self) {
        lock(&self.shared).phase = TimelineState::Closed;
        self.end_playback(PlaybackOutcome::Closed);
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.finish();
        for reply in self.acknowledgements.drain(..) { let _ = reply.send(Err(closed_error())); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchored_clock_reverses_and_changes_speed_without_jumps() {
        let now = Instant::now();
        let mut clock = Clock::new(2.0, now);
        clock.destination = Some(0.5);
        assert_eq!(clock.position_at(now + Duration::from_secs(3)), 0.5);
        clock.reanchor(now + Duration::from_millis(500));
        assert_eq!(clock.anchor, 0.25);
        clock.speed = 2.0;
        assert_eq!(clock.position_at(now + Duration::from_millis(750)), 0.5);
        clock.reanchor(now + Duration::from_millis(750));
        clock.destination = Some(0.0);
        assert_eq!(clock.position_at(now + Duration::from_secs(1)), 0.25);
        clock.reanchor(now + Duration::from_secs(1));
        clock.destination = None;
        assert_eq!(clock.position_at(now + Duration::from_secs(100)), 0.25);
    }

    #[test]
    fn rejects_invalid_controls() {
        for value in [f64::NAN, f64::INFINITY, -0.1, 1.1] { assert!(validate_position(value).is_err()); }
        for value in [f64::NAN, f64::INFINITY, -1.0, 0.0] { assert!(validate_speed(value).is_err()); }
    }
}
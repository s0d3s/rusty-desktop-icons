//! End-to-end animation tests driven through a [`FakeDesktop`] backend.
//!
//! These tests run at real time but use very short durations (~50 ms per
//! animation) to keep the whole test suite quick.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use rdi_core::fake::FakeDesktop;
use rdi_core::{
    AnimationOptions, Curve, DesktopController, DesktopError, Duration, FinishReason,
    IconAnimationSpec, IconId, IconSnapshot, Point, PreObservers, StopMode,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn snap(id: &str, x: i32, y: i32) -> IconSnapshot {
    IconSnapshot::new(IconId::from(id), id, None, false, Point::new(x, y))
}

fn linear_spec(id: &str, target: (i32, i32), millis: u64) -> IconAnimationSpec {
    IconAnimationSpec::new(
        IconId::from(id),
        Point::new(target.0, target.1),
        Duration::fixed(StdDuration::from_millis(millis)),
        Curve::linear(),
    )
}

/// A modest "expected within 5 s" bound: animations are ~50 ms but CI can
/// stall. The wait_timeout call itself provides the real safeguard.
const WAIT_TIMEOUT: StdDuration = StdDuration::from_secs(5);

/// Test-side default options.
///
/// The library ships with a slow 5 Hz tick rate to work around a
/// Windows 11 shell repaint limitation. Tests run against
/// [`FakeDesktop`] (no real shell), so they can safely go much faster
/// — 100 Hz keeps the full suite under a couple of seconds.
fn test_options() -> AnimationOptions {
    AnimationOptions {
        tick_hz: Some(100),
        ..Default::default()
    }
}

#[test]
fn timeline_visibility_preserves_playback_and_cleanup() {
    use rdi_core::{PlaybackOutcome, TimelineCloseMode, TimelineState};
    for drop_session in [false, true] {
        let desktop = FakeDesktop::new();
        desktop.add_icon(snap("icon", 0, 0));
        let controller = DesktopController::new(desktop.clone()).unwrap();
        let timeline = controller.prepare(vec![linear_spec("icon", (100, 100), 30_000)], test_options())
            .unwrap().open_timeline().unwrap();
        assert!(!timeline.real_icons_visible());
        let playback = timeline.play_to(1.0, 0.001).unwrap();
        for visible in [true, false, true, false] {
            timeline.set_real_icons_visible(visible).unwrap();
            assert_eq!(timeline.real_icons_visible(), visible);
            assert_eq!(desktop.real_icons_visible(), visible);
            assert_eq!(timeline.state(), TimelineState::Playing);
            assert_eq!(playback.wait_timeout(StdDuration::ZERO).unwrap(), None);
            assert_eq!(timeline.speed(), 0.001);
            assert!(matches!(controller.apply_flags(0x200, 0), Err(DesktopError::AnimationBusy)));
        }
        assert!(desktop.commit_log().is_empty());
        assert!(desktop.overlay_finalize_log().is_empty());
        if !drop_session { timeline.close(TimelineCloseMode::RestoreOrigins).unwrap(); }
        drop(timeline);
        controller.shutdown();
        assert!(desktop.real_icons_visible());
        assert_eq!(playback.wait_timeout(WAIT_TIMEOUT).unwrap(), Some(PlaybackOutcome::Closed));
        assert_eq!(desktop.overlay_finalize_log().len(), 1);
        assert_eq!(desktop.position_of(&IconId::from("icon")), Some(Point::new(0, 0)));
    }
}

#[test]
fn grid_targets_are_resolved_before_preparation_and_shared_by_timeline() {
    let desktop = FakeDesktop::new();
    let bounds = rdi_core::Rect::new(0, 0, 400, 200);
    let info = rdi_core::DesktopInfo { bounds, monitors: vec![], grids: vec![
        rdi_core::IconGrid::new("test".into(), bounds, Point::new(48, 48),
            Point::new(100, 100), Some(Point::new(0, 0)), false).unwrap(),
    ] };
    desktop.set_desktop_info(info.clone());
    desktop.add_icon(snap("first", 0, 0));
    desktop.add_icon(snap("second", 100, 0));
    desktop.add_icon(snap("stationary", 200, 0));
    let controller = DesktopController::new(desktop.clone()).unwrap();
    assert_eq!(controller.desktop_info().unwrap(), info);
    let options = AnimationOptions { snap_to_grid: true, ..test_options() };
    let prepared = controller.prepare(vec![linear_spec("first", (199, 2), 10),
        linear_spec("second", (199, 2), 10)], options).unwrap();
    assert!(desktop.commit_log().is_empty());
    assert!(desktop.flag_ops().is_empty());
    assert_eq!(controller.desktop_info().unwrap(), info);
    let plans = desktop.last_overlay_plans();
    assert_eq!(plans[0].final_position, Point::new(200, 100));
    assert_eq!(plans[1].final_position, Point::new(100, 0));
    let timeline = prepared.open_timeline().unwrap();
    assert_eq!(controller.desktop_info().unwrap(), info);
    timeline.seek(1.0).unwrap();
    assert_eq!(timeline.snapshot()[0].current, plans[0].final_position);
    timeline.close(rdi_core::TimelineCloseMode::TeleportToTarget).unwrap();
    assert_eq!(desktop.position_of(&IconId::from("first")), Some(plans[0].final_position));
    assert_eq!(desktop.position_of(&IconId::from("stationary")), Some(Point::new(200, 0)));
}

#[test]
fn grid_distance_duration_uses_resolved_target_and_missing_ids_remain_reported() {
    let desktop = FakeDesktop::new();
    let bounds = rdi_core::Rect::new(0, 0, 400, 200);
    desktop.set_desktop_info(rdi_core::DesktopInfo { bounds, monitors: vec![], grids: vec![
        rdi_core::IconGrid::new("test".into(), bounds, Point::new(48, 48),
            Point::new(100, 100), Some(Point::new(0, 0)), false).unwrap(),
    ] });
    desktop.add_icon(snap("distance", 0, 0));
    desktop.add_icon(snap("clock", 0, 100));
    let controller = DesktopController::new(desktop.clone()).unwrap();
    let options = AnimationOptions { snap_to_grid: true, ..test_options() };
    let mut distance = linear_spec("distance", (149, 0), 1);
    distance.duration = Duration::distance(100.0);
    let timeline = controller.prepare(vec![distance, linear_spec("clock", (300, 100), 2000),
        linear_spec("missing", (100, 0), 1)], options).unwrap().open_timeline().unwrap();
    timeline.seek(0.25).unwrap();
    let state = timeline.snapshot();
    assert_eq!(state[0].target, Point::new(100, 0));
    assert_eq!(state[0].t, 0.5);
    assert_eq!(state[0].current, Point::new(50, 0));
    assert_eq!(timeline.missing_icons(), vec![IconId::from("missing")]);
    timeline.close(rdi_core::TimelineCloseMode::RestoreOrigins).unwrap();

    let controller = DesktopController::new(FakeDesktop::new()).unwrap();
    let handle = controller.animate(vec![linear_spec("missing", (1, 1), 1)], options).unwrap();
    assert!(matches!(handle.wait_timeout(WAIT_TIMEOUT), Some(FinishReason::Completed)));
    assert_eq!(handle.missing_icons(), vec![IconId::from("missing")]);
}

#[test]
fn grid_full_fails_before_any_flags_overlay_or_writes() {
    let desktop = FakeDesktop::new();
    let bounds = rdi_core::Rect::new(0, 0, 100, 100);
    desktop.set_desktop_info(rdi_core::DesktopInfo { bounds, monitors: vec![], grids: vec![
        rdi_core::IconGrid::new("tiny".into(), bounds, Point::new(48, 48),
            Point::new(100, 100), Some(Point::new(0, 0)), false).unwrap(),
    ] });
    desktop.add_icon(snap("moving", 200, 0));
    desktop.add_icon(snap("fixed", 0, 0));
    let controller = DesktopController::new(desktop.clone()).unwrap();
    let options = AnimationOptions { snap_to_grid: true,
        before_flags: Some(rdi_core::FolderFlagOp::Set(4)), ..test_options() };
    assert!(controller.prepare(vec![linear_spec("moving", (2, 2), 10)], options).is_err());
    let handle = controller.animate(vec![linear_spec("moving", (2, 2), 10)], options).unwrap();
    assert!(matches!(handle.wait_timeout(WAIT_TIMEOUT), Some(FinishReason::Error(_))));
    assert!(desktop.commit_log().is_empty());
    assert!(desktop.flag_ops().is_empty());
    assert!(desktop.last_overlay_plans().is_empty());
    let handle = controller.animate(vec![linear_spec("moving", (2, 2), 10)], test_options()).unwrap();
    assert!(matches!(handle.wait_timeout(WAIT_TIMEOUT), Some(FinishReason::Completed)));
    assert_eq!(desktop.position_of(&IconId::from("moving")), Some(Point::new(2, 2)));
}

#[test]
fn timeline_rewinds_completed_tracks_without_finalizing() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("short", 0, 0));
    desktop.add_icon(snap("long", 0, 10));
    let controller = DesktopController::new(desktop.clone()).unwrap();
    let timeline = controller.prepare(vec![
        linear_spec("short", (1000, 0), 50),
        linear_spec("long", (1000, 10), 100),
    ], test_options()).unwrap().open_timeline().unwrap();
    assert_eq!(timeline.state(), rdi_core::TimelineState::Paused);
    assert_eq!(timeline.position(), 0.0);
    timeline.seek(1.0).unwrap();
    assert!(desktop.overlay_finalize_log().is_empty());
    timeline.seek(0.25).unwrap();
    assert_eq!(timeline.snapshot()[0].current, Point::new(500, 0));
    assert_eq!(timeline.snapshot()[1].current, Point::new(250, 10));
    for destination in [0.5, 0.0, 1.0, 0.0] {
        let playback = timeline.play_to(destination, 2.0).unwrap();
        assert_eq!(playback.wait_timeout(WAIT_TIMEOUT).unwrap(), Some(rdi_core::PlaybackOutcome::Reached));
        assert_eq!(timeline.position(), destination);
        assert_eq!(timeline.state(), rdi_core::TimelineState::Paused);
        assert!(desktop.overlay_finalize_log().is_empty());
    }
    assert!(matches!(controller.set_positions(vec![]), Err(DesktopError::AnimationBusy)));
    timeline.close(rdi_core::TimelineCloseMode::RestoreOrigins).unwrap();
    assert_eq!(desktop.overlay_finalize_log().len(), 1);
    assert_eq!(desktop.position_of(&IconId::from("long")), Some(Point::new(0, 10)));
}

#[test]
fn timeline_pause_freezes_visual_clock_and_waits_are_repeatable() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("icon", 0, 0));
    let controller = DesktopController::new(desktop.clone()).unwrap();
    let timeline = controller.prepare(vec![linear_spec("icon", (1000, 0), 1000)], test_options())
        .unwrap().open_timeline().unwrap();
    timeline.seek(0.5).unwrap();
    let first = desktop.visual_frame_log().last().unwrap()[0].clone();
    let count = desktop.visual_frame_log().len();
    std::thread::sleep(StdDuration::from_millis(120));
    assert_eq!(desktop.visual_frame_log().len(), count);
    timeline.seek(1.0).unwrap();
    timeline.seek(0.5).unwrap();
    let repeated = desktop.visual_frame_log().last().unwrap()[0].clone();
    assert_eq!(first.position, repeated.position);
    assert_eq!(first.progress, repeated.progress);
    assert_eq!(first.elapsed_seconds, repeated.elapsed_seconds);
    assert_eq!(first.elapsed_seconds, 0.5);
    let playback = timeline.play_to(1.0, 0.01).unwrap();
    assert_eq!(playback.wait_timeout(StdDuration::ZERO).unwrap(), None);
    timeline.set_speed(0.02).unwrap();
    timeline.pause().unwrap();
    assert_eq!(playback.wait().unwrap(), rdi_core::PlaybackOutcome::Interrupted);
    assert_eq!(playback.wait().unwrap(), rdi_core::PlaybackOutcome::Interrupted);
    let current = timeline.snapshot()[0].current;
    timeline.close(rdi_core::TimelineCloseMode::LeaveInPlace).unwrap();
    assert_eq!(desktop.position_of(&IconId::from("icon")), Some(current));
    assert!(timeline.seek(0.0).is_err());
    timeline.close(rdi_core::TimelineCloseMode::RestoreOrigins).unwrap();
    assert_eq!(desktop.overlay_finalize_log().len(), 1);
}

#[test]
fn timeline_drop_shutdown_and_stale_handles_are_safe() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("icon", 0, 0));
    let controller = DesktopController::new(desktop.clone()).unwrap();
    let stale = controller.animate(vec![], test_options()).unwrap();
    stale.wait();
    let timeline = controller.prepare(vec![linear_spec("icon", (1000, 0), 100)], test_options())
        .unwrap().open_timeline().unwrap();
    stale.stop(StopMode::TeleportToTarget);
    timeline.seek(0.5).unwrap();
    drop(timeline);
    controller.shutdown();
    assert_eq!(desktop.position_of(&IconId::from("icon")), Some(Point::new(0, 0)));

    let controller = DesktopController::new(desktop.clone()).unwrap();
    let timeline = controller.prepare(vec![linear_spec("icon", (1000, 0), 100)], test_options())
        .unwrap().open_timeline().unwrap();
    let playback = timeline.play_to(1.0, 0.001).unwrap();
    controller.shutdown();
    assert_eq!(playback.wait().unwrap(), rdi_core::PlaybackOutcome::Closed);
    assert_eq!(timeline.state(), rdi_core::TimelineState::Closed);
    assert_eq!(desktop.position_of(&IconId::from("icon")), Some(Point::new(0, 0)));
}

#[test]
fn paused_timeline_detects_disruption_without_rendering() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("icon", 0, 0));
    let controller = DesktopController::new(desktop.clone()).unwrap();
    let timeline = controller.prepare(vec![linear_spec("icon", (1000, 0), 100)], test_options())
        .unwrap().open_timeline().unwrap();
    desktop.queue_commit_error(DesktopError::OverlayCancelled("display changed".into()));
    let deadline = std::time::Instant::now() + WAIT_TIMEOUT;
    while timeline.state() != rdi_core::TimelineState::Closed {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(timeline.finish_reason(), Some(FinishReason::Stopped(StopMode::TeleportToTarget)));
    assert_eq!(desktop.overlay_frame_log().len(), 1);
    assert_eq!(desktop.position_of(&IconId::from("icon")), Some(Point::new(1000, 0)));
}

#[test]
fn timeline_close_target_and_open_failure_release_the_reservation() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("icon", 0, 0));
    let controller = DesktopController::new(desktop.clone()).unwrap();
    let prepared = controller.prepare(vec![linear_spec("icon", (1000, 0), 100)], test_options()).unwrap();
    desktop.queue_commit_error(DesktopError::BackendUnavailable("first frame failed".into()));
    assert!(prepared.open_timeline().is_err());
    assert!(controller.set_positions(vec![]).is_ok());
    let timeline = controller.prepare(vec![linear_spec("icon", (1000, 0), 100)], test_options())
        .unwrap().open_timeline().unwrap();
    timeline.close(rdi_core::TimelineCloseMode::TeleportToTarget).unwrap();
    assert_eq!(timeline.state(), rdi_core::TimelineState::Closed);
    assert_eq!(desktop.position_of(&IconId::from("icon")), Some(Point::new(1000, 0)));
    let newer = controller.prepare(vec![], test_options()).unwrap().open_timeline().unwrap();
    assert!(timeline.seek(0.0).is_err());
    newer.seek(1.0).unwrap();
    assert_eq!(newer.state(), rdi_core::TimelineState::Paused);
    newer.close(rdi_core::TimelineCloseMode::RestoreOrigins).unwrap();
}

#[test]
fn preparation_does_not_move_or_consume_duration() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("prepared", 0, 0));
    let controller = DesktopController::new(desktop.clone()).unwrap();
    let prepared = controller.prepare(vec![linear_spec("prepared", (1000, 0), 120)], test_options()).unwrap();
    assert!(desktop.overlay_frame_log().is_empty());
    assert!(desktop.commit_log().is_empty());
    assert!(matches!(controller.set_positions(vec![]), Err(DesktopError::AnimationBusy)));
    std::thread::sleep(StdDuration::from_millis(180));
    assert!(desktop.overlay_frame_log().is_empty());
    let clock = std::time::Instant::now();
    let handle = prepared.start().unwrap();
    assert_eq!(handle.wait_timeout(WAIT_TIMEOUT), Some(FinishReason::Completed));
    assert!(clock.elapsed() >= StdDuration::from_millis(100));
    assert_eq!(desktop.overlay_frame_log()[0][0].1, Point::new(0, 0));
}

#[test]
fn cancelling_preparation_does_not_commit() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("prepared", 0, 0));
    let controller = DesktopController::new(desktop.clone()).unwrap();
    controller.prepare(vec![linear_spec("prepared", (1000, 0), 120)], test_options()).unwrap().cancel();
    assert!(desktop.overlay_finalize_log().is_empty());
    assert!(desktop.commit_log().is_empty());
    assert!(controller.set_positions(vec![]).is_ok());
}

#[test]
fn effect_timeline_survives_stationary_motion() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("stationary", 0, 0));
    let controller = DesktopController::new(desktop).unwrap();
    let mut spec = linear_spec("stationary", (0, 0), 120);
    spec.effect = Some(rdi_core::Effect {
        shader: rdi_core::ShaderProgram { execution: None, pixel_bytecode: Arc::from([1u8]), vertex_bytecode: Arc::from([1u8]), pipeline: rdi_core::ShaderPipeline::Sprite },
        params: [0.0; 4], padding_px: 0, envelope: Curve::linear(), seed: 0.0,
    });
    let clock = std::time::Instant::now();
    let handle = controller.animate(vec![spec], test_options()).unwrap();
    assert_eq!(handle.wait_timeout(WAIT_TIMEOUT), Some(FinishReason::Completed));
    assert!(clock.elapsed() >= StdDuration::from_millis(120));
}

#[test]
fn effect_timeline_keeps_initial_and_final_movement_holds() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("held", 0, 0));
    let controller = DesktopController::new(desktop.clone()).unwrap();
    let movement = Curve::keyframes(vec![
        rdi_core::Keyframe::new(0.0, 0.0), rdi_core::Keyframe::new(0.3, 0.0),
        rdi_core::Keyframe::new(0.6, 1.0), rdi_core::Keyframe::new(1.0, 1.0),
    ], rdi_core::KeyframeInterp::Linear).unwrap();
    let mut spec = IconAnimationSpec::new(IconId::from("held"), Point::new(1000, 0),
        Duration::fixed(StdDuration::from_millis(200)), movement);
    spec.effect = Some(rdi_core::Effect {
        shader: rdi_core::ShaderProgram { execution: None, pixel_bytecode: Arc::from([1u8]), vertex_bytecode: Arc::from([1u8]), pipeline: rdi_core::ShaderPipeline::Sprite },
        params: [0.0; 4], padding_px: 0, envelope: Curve::linear(), seed: 0.0,
    });
    let clock = std::time::Instant::now();
    let handle = controller.animate(vec![spec], test_options()).unwrap();
    assert_eq!(handle.wait_timeout(WAIT_TIMEOUT), Some(FinishReason::Completed));
    assert!(clock.elapsed() >= StdDuration::from_millis(200));
    let frames = desktop.overlay_frame_log();
    assert!(frames.iter().filter(|frame| frame[0].1 == Point::new(0, 0)).count() >= 2);
    assert!(frames.iter().filter(|frame| frame[0].1 == Point::new(1000, 0)).count() >= 2);
}

#[test]
fn dropping_preparation_and_controller_never_commits() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("prepared", 0, 0));
    let controller = DesktopController::new(desktop.clone()).unwrap();
    let prepared = controller.prepare(vec![linear_spec("prepared", (1000, 0), 120)], test_options()).unwrap();
    drop(prepared);
    drop(controller);
    assert!(desktop.overlay_finalize_log().is_empty());
    assert!(desktop.commit_log().is_empty());
}

// ---------------------------------------------------------------------------
// Baseline: an animation that completes successfully
// ---------------------------------------------------------------------------

#[test]
fn animation_completes_and_leaves_icons_at_target() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("a", 0, 0));
    desktop.add_icon(snap("b", 100, 100));

    let ctrl = DesktopController::new(desktop.clone()).unwrap();

    let handle = ctrl
        .animate(
            vec![
                linear_spec("a", (200, 0), 50),
                linear_spec("b", (100, 300), 50),
            ],
            test_options(),
        )
        .unwrap();

    let reason = handle.wait_timeout(WAIT_TIMEOUT).expect("finished in time");
    assert_eq!(reason, FinishReason::Completed);
    assert!(!handle.is_running());
    assert_eq!(handle.missing_icons(), Vec::<IconId>::new());

    // Final positions must equal targets.
    assert_eq!(desktop.position_of(&IconId::from("a")), Some(Point::new(200, 0)));
    assert_eq!(desktop.position_of(&IconId::from("b")), Some(Point::new(100, 300)));

    // With the overlay architecture the shell is not touched per
    // tick — the engine commits into `commit_overlay_frame` and
    // hands the final positions to `finalize_overlay_session` once.
    let frames = desktop.overlay_frame_log().len();
    assert!(frames >= 2, "expected \u{2265}2 overlay frames, got {frames}");
    assert_eq!(
        desktop.overlay_finalize_log().len(),
        1,
        "finalize_overlay_session must fire exactly once per animation",
    );
}

// ---------------------------------------------------------------------------
// Missing icons are reported, not fatal
// ---------------------------------------------------------------------------

#[test]
fn missing_icons_are_reported_and_others_still_animate() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("real", 0, 0));

    let ctrl = DesktopController::new(desktop.clone()).unwrap();

    let handle = ctrl
        .animate(
            vec![
                linear_spec("real", (50, 0), 50),
                linear_spec("ghost", (999, 999), 50),
            ],
            test_options(),
        )
        .unwrap();

    let reason = handle.wait_timeout(WAIT_TIMEOUT).expect("finished in time");
    assert_eq!(reason, FinishReason::Completed);
    assert_eq!(handle.missing_icons(), vec![IconId::from("ghost")]);
    assert_eq!(
        desktop.position_of(&IconId::from("real")),
        Some(Point::new(50, 0))
    );
}

// ---------------------------------------------------------------------------
// Snap-to-grid is no longer the engine's business
// ---------------------------------------------------------------------------

#[test]
fn engine_never_touches_snap_to_grid() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("a", 0, 0));
    {
        let mut b = desktop.clone();
        use rdi_core::DesktopBackend;
        b.apply_flags(0x4, 0xFFFF_FFFF).unwrap();
    }
    assert_eq!(desktop.flags() & 0x4, 0x4);

    let ctrl = DesktopController::new(desktop.clone()).unwrap();
    let handle = ctrl
        .animate(
            vec![linear_spec("a", (10, 0), 30)],
            test_options(),
        )
        .unwrap();
    handle.wait_timeout(WAIT_TIMEOUT).unwrap();

    // The flag is left exactly as the caller had it, and the only
    // logged op is this test's own setup call — the engine issued none.
    // `before_flags` / `after_flags` are both None here.
    assert_eq!(desktop.flags() & 0x4, 0x4);
    assert_eq!(
        desktop.flag_ops(),
        vec![(0x4, 0xFFFF_FFFF)],
        "engine issued unexpected flag ops"
    );
}

// ---------------------------------------------------------------------------
// Stop modes
// ---------------------------------------------------------------------------

#[test]
fn stop_leave_in_place_keeps_icons_where_they_are() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("a", 0, 0));

    let ctrl = DesktopController::new(desktop.clone()).unwrap();
    let handle = ctrl
        .animate(
            // Long enough to be caught mid-flight.
            vec![linear_spec("a", (500, 0), 500)],
            test_options(),
        )
        .unwrap();

    // Let it run for a moment.
    std::thread::sleep(StdDuration::from_millis(60));
    handle.stop(StopMode::LeaveInPlace);

    let reason = handle.wait_timeout(WAIT_TIMEOUT).expect("finished");
    assert_eq!(reason, FinishReason::Stopped(StopMode::LeaveInPlace));

    // Icon should be somewhere between origin and target — NOT at 500.
    let pos = desktop.position_of(&IconId::from("a")).unwrap();
    assert!(
        pos.x > 0 && pos.x < 500,
        "expected mid-flight position, got {pos:?}"
    );
}

#[test]
fn stop_teleport_snaps_icons_to_target() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("a", 0, 0));

    let ctrl = DesktopController::new(desktop.clone()).unwrap();
    let handle = ctrl
        .animate(
            vec![linear_spec("a", (500, 0), 1000)],
            test_options(),
        )
        .unwrap();

    std::thread::sleep(StdDuration::from_millis(30));
    handle.stop(StopMode::TeleportToTarget);

    let reason = handle.wait_timeout(WAIT_TIMEOUT).expect("finished");
    assert_eq!(reason, FinishReason::Stopped(StopMode::TeleportToTarget));
    assert_eq!(
        desktop.position_of(&IconId::from("a")),
        Some(Point::new(500, 0))
    );
}

// ---------------------------------------------------------------------------
// Observers
// ---------------------------------------------------------------------------

#[test]
fn observers_fire_in_expected_order() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("a", 0, 0));
    desktop.add_icon(snap("b", 0, 0));

    let ctrl = DesktopController::new(desktop.clone()).unwrap();

    let started = Arc::new(AtomicUsize::new(0));
    let ticks = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));

    // Pre-attach observers atomically for exact counts regardless of
    // scheduling — see `PreObservers` docs.
    let pre = {
        let started = started.clone();
        let ticks = ticks.clone();
        let completed = completed.clone();
        let finished = finished.clone();
        PreObservers::new()
            .on_start(move |_| {
                started.fetch_add(1, Ordering::Relaxed);
            })
            .on_tick(move |_| {
                ticks.fetch_add(1, Ordering::Relaxed);
            })
            .on_icon_complete(move |_| {
                completed.fetch_add(1, Ordering::Relaxed);
            })
            .on_finish(move |_| {
                finished.fetch_add(1, Ordering::Relaxed);
            })
    };

    let handle = ctrl
        .animate_with_observers(
            vec![
                linear_spec("a", (10, 0), 40),
                linear_spec("b", (0, 10), 40),
            ],
            test_options(),
            pre,
        )
        .unwrap();

    handle.wait_timeout(WAIT_TIMEOUT).unwrap();

    assert_eq!(started.load(Ordering::Relaxed), 1, "on_start fires once");
    assert!(ticks.load(Ordering::Relaxed) >= 1, "no ticks observed");
    assert_eq!(
        completed.load(Ordering::Relaxed),
        2,
        "expected 2 icon completions"
    );
    assert_eq!(finished.load(Ordering::Relaxed), 1);
}

// ---------------------------------------------------------------------------
// Multi-monitor coordinate support
// ---------------------------------------------------------------------------

#[test]
fn list_monitors_round_trips_through_worker() {
    use rdi_core::{MonitorInfo, Rect};

    let desktop = FakeDesktop::new();
    desktop.set_monitors(vec![
        MonitorInfo {
            id: "PRIMARY".into(),
            name: "PRIMARY".into(),
            bounds: Rect::from_origin_size(0, 0, 1920, 1080),
            work_area: Rect::from_origin_size(0, 0, 1920, 1040),
            is_primary: true,
            scale_factor: 1.0,
        },
        MonitorInfo {
            id: "LEFT".into(),
            name: "LEFT".into(),
            // Second monitor arranged to the LEFT of the primary → negative x.
            bounds: Rect::from_origin_size(-1920, 0, 1920, 1080),
            work_area: Rect::from_origin_size(-1920, 0, 1920, 1040),
            is_primary: false,
            scale_factor: 1.5,
        },
    ]);

    let ctrl = DesktopController::new(desktop.clone()).unwrap();
    let monitors = ctrl.list_monitors().unwrap();
    assert_eq!(monitors.len(), 2);

    let primary = monitors.iter().find(|m| m.is_primary).expect("no primary");
    assert_eq!(primary.bounds.left, 0);
    assert_eq!(primary.bounds.width(), 1920);

    let secondary = monitors.iter().find(|m| !m.is_primary).expect("no secondary");
    assert_eq!(secondary.bounds.left, -1920);
    assert!((secondary.scale_factor - 1.5).abs() < 1e-6);
}

#[test]
fn animation_accepts_negative_coordinates() {
    // Verify the entire pipeline (controller → engine → backend →
    // commit) is happy with virtual-screen coordinates that live on a
    // monitor arranged to the left of the primary.
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("a", 100, 100));

    let ctrl = DesktopController::new(desktop.clone()).unwrap();
    let handle = ctrl
        .animate(
            vec![linear_spec("a", (-500, -50), 40)],
            test_options(),
        )
        .unwrap();
    let reason = handle.wait_timeout(WAIT_TIMEOUT).unwrap();
    assert!(matches!(reason, FinishReason::Completed));

    // The fake backend records exactly what the engine committed.
    let final_pos = desktop.position_of(&IconId::from("a")).unwrap();
    assert_eq!(final_pos, Point::new(-500, -50));
}

#[test]
fn set_positions_accepts_negative_coordinates() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("a", 0, 0));

    let ctrl = DesktopController::new(desktop.clone()).unwrap();
    let missing = ctrl
        .set_positions(vec![(IconId::from("a"), Point::new(-100, -200))])
        .unwrap();
    assert!(missing.is_empty());
    assert_eq!(
        desktop.position_of(&IconId::from("a")).unwrap(),
        Point::new(-100, -200)
    );
}

// ---------------------------------------------------------------------------
// Direct positioning (no animation)
// ---------------------------------------------------------------------------

#[test]
fn set_positions_moves_icons_and_reports_missing() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("a", 0, 0));

    let ctrl = DesktopController::new(desktop.clone()).unwrap();
    let missing = ctrl
        .set_positions(vec![
            (IconId::from("a"), Point::new(42, 42)),
            (IconId::from("nope"), Point::new(0, 0)),
        ])
        .unwrap();
    assert_eq!(missing, vec![IconId::from("nope")]);
    assert_eq!(
        desktop.position_of(&IconId::from("a")),
        Some(Point::new(42, 42))
    );
}

// ---------------------------------------------------------------------------
// Flag helpers
// ---------------------------------------------------------------------------

#[test]
fn flag_helpers_have_expected_semantics() {
    let desktop = FakeDesktop::new();
    let ctrl = DesktopController::new(desktop.clone()).unwrap();

    // set_flags OR's bits in.
    ctrl.set_flags(0b0101).unwrap();
    assert_eq!(ctrl.get_flags().unwrap(), 0b0101);

    // unset_flags clears the given bits.
    ctrl.unset_flags(0b0001).unwrap();
    assert_eq!(ctrl.get_flags().unwrap(), 0b0100);

    // toggle_flags XORs.
    ctrl.toggle_flags(0b0110).unwrap();
    assert_eq!(ctrl.get_flags().unwrap(), 0b0010);

    // set_flags_exactly sets bits up to the highest requested bit.
    ctrl.set_flags_exactly(0b0101).unwrap();
    // build_true_mask(0b0101) == 0b0111; new = (2 & !7) | (5 & 7) = 0 | 5 = 5
    assert_eq!(ctrl.get_flags().unwrap(), 0b0101);
}

// ---------------------------------------------------------------------------
// Mid-animation Shell access
// ---------------------------------------------------------------------------

#[test]
fn shell_writes_are_refused_while_an_animation_runs() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("a", 0, 0));

    let ctrl = DesktopController::new(desktop.clone()).unwrap();
    let handle = ctrl
        .animate(vec![linear_spec("a", (500, 0), 500)], test_options())
        .unwrap();
    std::thread::sleep(StdDuration::from_millis(60));

    assert!(matches!(
        ctrl.set_positions(vec![(IconId::from("a"), Point::new(7, 7))]),
        Err(DesktopError::AnimationBusy)
    ));
    assert!(matches!(
        ctrl.apply_flags(0x4, 0xFFFF_FFFF),
        Err(DesktopError::AnimationBusy)
    ));

    // Reads stay available — they cannot corrupt the overlay session.
    assert!(ctrl.list_icons().is_ok());
    assert!(ctrl.get_flags().is_ok());
    assert!(ctrl.list_monitors().is_ok());

    handle.stop(StopMode::TeleportToTarget);
    handle.wait_timeout(WAIT_TIMEOUT).expect("finished");

    // The refused write never reached the backend.
    assert_eq!(desktop.flag_ops(), Vec::<(u32, u32)>::new());
    assert_eq!(
        desktop.position_of(&IconId::from("a")),
        Some(Point::new(500, 0))
    );

    // Writes work again once the animation is over.
    ctrl.apply_flags(0x4, 0xFFFF_FFFF).unwrap();
}

// ---------------------------------------------------------------------------
// Error path: list_icons failure during animation start
// ---------------------------------------------------------------------------

#[test]
fn animate_reports_backend_error_via_finish_reason() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("a", 0, 0));
    desktop.set_next_list_error(rdi_core::DesktopError::BackendUnavailable("boom".into()));

    let ctrl = DesktopController::new(desktop.clone()).unwrap();
    let handle = ctrl
        .animate(
            vec![linear_spec("a", (10, 0), 20)],
            test_options(),
        )
        .unwrap();

    let reason = handle.wait_timeout(WAIT_TIMEOUT).unwrap();
    match reason {
        FinishReason::Error(msg) => assert!(msg.contains("boom"), "msg = {msg}"),
        other => panic!("expected Error(_), got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Cancellation path: OverlayCancelled → graceful teleport stop
// ---------------------------------------------------------------------------

#[test]
fn overlay_cancelled_finishes_as_stopped_teleport_and_still_finalises() {
    // Simulates the "Explorer restarted mid-animation" case: the
    // backend returns `OverlayCancelled` from a commit; the engine
    // should NOT surface this as an error, but end the animation
    // gracefully with `Stopped(TeleportToTarget)`, snap the in-
    // memory state to targets, AND still run
    // `finalize_overlay_session` for cleanup.
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("a", 0, 0));

    // Let the first commit succeed (the first-frame teleport
    // path); cancel on the second.
    desktop.queue_commit_error(rdi_core::DesktopError::OverlayCancelled(
        "Explorer restarted (test)".into(),
    ));

    let ctrl = DesktopController::new(desktop.clone()).unwrap();
    // Long-ish duration so the tick loop reaches at least one
    // commit before finish.
    let handle = ctrl
        .animate(
            vec![linear_spec("a", (500, 0), 500)],
            test_options(),
        )
        .unwrap();

    let reason = handle.wait_timeout(WAIT_TIMEOUT).unwrap();
    assert_eq!(
        reason,
        FinishReason::Stopped(StopMode::TeleportToTarget),
        "OverlayCancelled must map to graceful teleport stop",
    );

    // The fake `finalize_overlay_session` moves icons to the target
    // positions it receives. If the engine ran finalize, icon "a"
    // ends up at (500, 0). If it skipped finalize (bug), the
    // position would be somewhere between origin and target.
    assert_eq!(
        desktop.position_of(&IconId::from("a")),
        Some(Point::new(500, 0)),
        "engine must still run finalize_overlay_session after cancel",
    );
    assert_eq!(
        desktop.overlay_finalize_log().len(),
        1,
        "finalize should fire exactly once even on cancellation",
    );
}

// ---------------------------------------------------------------------------
// Final-commit outcome
//
// The backend runs an up-to-250 ms Shell confirmation poll to produce
// the outcome, so the engine must surface it rather than discard it.
// ---------------------------------------------------------------------------

#[test]
fn final_commit_outcome_is_surfaced_on_the_handle() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("a", 0, 0));
    desktop.add_icon(snap("b", 100, 100));

    let ctrl = DesktopController::new(desktop.clone()).unwrap();
    let handle = ctrl
        .animate(
            vec![
                linear_spec("a", (200, 0), 50),
                linear_spec("b", (100, 300), 50),
            ],
            test_options(),
        )
        .unwrap();
    handle.wait_timeout(WAIT_TIMEOUT).expect("animation timed out");

    let outcome = handle
        .final_commit()
        .expect("a completed overlay animation must report its commit outcome");
    let mut moved = outcome.moved_ids.clone();
    moved.sort();
    assert_eq!(
        moved,
        vec![IconId::from("a"), IconId::from("b")],
        "both icons resolved, so both are confirmed by the fake backend",
    );
    assert!(
        outcome.missing_ids.is_empty(),
        "nothing vanished mid-animation: {:?}",
        outcome.missing_ids,
    );
}

#[test]
fn final_commit_reports_icons_that_vanished_mid_animation() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("stays", 0, 0));
    desktop.add_icon(snap("vanishes", 100, 100));

    let ctrl = DesktopController::new(desktop.clone()).unwrap();
    let handle = ctrl
        .animate(
            vec![
                linear_spec("stays", (200, 0), 120),
                linear_spec("vanishes", (100, 300), 120),
            ],
            test_options(),
        )
        .unwrap();

    // Pull the icon out from under the running animation, the way a
    // user deleting a desktop file would.
    std::thread::sleep(StdDuration::from_millis(30));
    desktop.remove_icon(&IconId::from("vanishes"));

    handle.wait_timeout(WAIT_TIMEOUT).expect("animation timed out");

    let outcome = handle.final_commit().expect("outcome must be reported");
    assert_eq!(
        outcome.missing_ids,
        vec![IconId::from("vanishes")],
        "the removed icon must be reported as unresolvable at commit time",
    );
    assert_eq!(
        outcome.moved_ids,
        vec![IconId::from("stays")],
        "the surviving icon still lands",
    );
}

#[test]
fn final_commit_is_none_while_the_animation_is_still_running() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("a", 0, 0));

    let ctrl = DesktopController::new(desktop.clone()).unwrap();
    let handle = ctrl
        .animate(vec![linear_spec("a", (500, 0), 400)], test_options())
        .unwrap();

    assert!(
        handle.final_commit().is_none(),
        "no commit has happened yet",
    );

    handle.stop(StopMode::TeleportToTarget);
    handle.wait_timeout(WAIT_TIMEOUT).expect("animation timed out");
    assert!(
        handle.final_commit().is_some(),
        "a stopped animation still finalises, so the outcome is known",
    );
}

#[test]
fn final_commit_is_reported_on_the_overlay_unavailable_fallback_path() {
    let desktop = FakeDesktop::new();
    desktop.add_icon(snap("a", 0, 0));

    let ctrl = DesktopController::new(desktop.clone()).unwrap();
    let handle = ctrl
        .animate(
            vec![linear_spec("a", (200, 0), 50)],
            AnimationOptions {
                force_fallback: true,
                ..test_options()
            },
        )
        .unwrap();
    handle.wait_timeout(WAIT_TIMEOUT).expect("animation timed out");

    let outcome = handle
        .final_commit()
        .expect("the fallback path commits too, so it reports an outcome");
    assert!(
        outcome.moved_ids.is_empty(),
        "the fallback path runs no confirmation poll, so it confirms nothing",
    );
    assert!(outcome.missing_ids.is_empty());
}

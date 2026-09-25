//! Diagnostic bin — loops the raw overlay session lifecycle until
//! Ctrl+C, wiggling every icon a few pixels right and back each cycle.
//!
//! Runs on the main STA thread (same pattern as
//! [`animate_direct`](animate_direct.rs)), calling `WindowsBackend`
//! directly instead of going through `DesktopController`. The point is
//! to isolate `begin_overlay_session` → `commit_overlay_frame` →
//! `finalize_overlay_session` from the engine tick loop so any
//! lifecycle glitch surfaces without ambiguity.
//!
//! Per cycle:
//!   1. `begin_overlay_session` — pre-renders overlay at current
//!      positions.
//!   2. `commit_overlay_frame(cur)` — first commit: teleports real
//!      icons to `final_position == cur` (no-op) + hides them + shows
//!      overlay.
//!   3. `commit_overlay_frame(cur + shift)` — overlay wiggles right.
//!   4. `commit_overlay_frame(cur)` — overlay wiggles back.
//!   5. Sleep.
//!   6. `finalize_overlay_session(cur)` — restores real-icon
//!      visibility; overlay window destroyed.
//!
//! On Ctrl+C the currently-running cycle completes normally before
//! the loop exits so `FWF_HIDEICONS` + hidden `SysListView32` are
//! always restored.
//!
//! Run with:
//!
//! ```powershell
//! cargo run --release --bin overlay_wiggle
//! ```
//!
//! Windows-only.

#![cfg(windows)]

use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration as StdDuration;

use rand::RngExt;
use rdi_core::{
    DesktopBackend, IconId, IconRenderPlan, OverlayRenderOptions, Point,
};
use rdi_platform_windows::WindowsBackend;

#[path = "../tracing_init.rs"]
mod tracing_init;

const SHIFT_PX_MIN: i32 = 10;
const SHIFT_PX_MAX: i32 = 20;

/// Pause after the overlay is first shown, before the right-shift.
const HOLD_AT_ORIGIN: StdDuration = StdDuration::from_millis(300);
/// Pause with the overlay shifted right.
const HOLD_AT_SHIFT: StdDuration = StdDuration::from_millis(400);
/// Pause after the overlay is back at origin, before finalizing.
const HOLD_BEFORE_FINALIZE: StdDuration = StdDuration::from_millis(1000);
/// Pause between cycles — real icons are visible during this gap.
const HOLD_BETWEEN_CYCLES: StdDuration = StdDuration::from_millis(500);

fn build_plans(icons: &[(IconId, Point)]) -> Vec<IconRenderPlan> {
    icons
        .iter()
        .map(|(id, pos)| IconRenderPlan::placeholder(id.clone(), *pos, *pos))
        .collect()
}

fn positions_at(
    icons: &[(IconId, Point)],
    shift_x: i32,
) -> Vec<(IconId, Point)> {
    icons
        .iter()
        .map(|(id, pos)| (id.clone(), Point::new(pos.x + shift_x, pos.y)))
        .collect()
}

/// One full begin → wiggle → finalize cycle. Always drives finalize,
/// even if a commit fails mid-cycle, so the real icons come back.
fn run_cycle(
    backend: &mut WindowsBackend,
    icons: &[(IconId, Point)],
    shift_x: i32,
    cycle: u32,
) -> Result<(), Box<dyn Error>> {
    let plans = build_plans(icons);
    let render_options = OverlayRenderOptions::all_enabled();
    let origin = positions_at(icons, 0);
    let shifted = positions_at(icons, shift_x);

    backend.begin_overlay_session(&plans, render_options)?;

    // Every path below MUST reach finalize, otherwise FWF_HIDEICONS
    // and the hidden SysListView32 leak.
    let wiggle = (|| -> Result<(), Box<dyn Error>> {
        // First commit: shows overlay + hides real icons. Positions
        // == current, so no visible jump.
        backend.commit_overlay_frame(&origin)?;
        std::thread::sleep(HOLD_AT_ORIGIN);

        // Shift right.
        backend.commit_overlay_frame(&shifted)?;
        std::thread::sleep(HOLD_AT_SHIFT);

        // Back to origin.
        backend.commit_overlay_frame(&origin)?;
        std::thread::sleep(HOLD_BEFORE_FINALIZE);
        Ok(())
    })();

    let finalize_result = backend.finalize_overlay_session(&origin);

    match (wiggle, finalize_result) {
        (Ok(()), Ok(outcome)) => {
            let missing = outcome.missing_ids.len();
            let note = if missing > 0 {
                format!(" ({missing} missing)")
            } else {
                String::new()
            };
            println!(
                "  cycle {cycle}: shift=+{shift_x}px, moved={}{note}",
                outcome.moved_ids.len()
            );
            Ok(())
        }
        (Err(e), Ok(_)) => {
            eprintln!("  cycle {cycle}: wiggle failed: {e}");
            Err(e)
        }
        (Ok(()), Err(e)) => {
            eprintln!("  cycle {cycle}: finalize failed: {e}");
            Err(Box::new(e))
        }
        (Err(w), Err(f)) => {
            eprintln!(
                "  cycle {cycle}: wiggle AND finalize failed: wiggle={w}, finalize={f}"
            );
            Err(w)
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_init::init();
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = Arc::clone(&stop);
        ctrlc::set_handler(move || {
            // Best-effort — main thread checks the flag at cycle
            // boundaries and exits after the running cycle unwinds
            // through `finalize_overlay_session`.
            stop.store(true, Ordering::SeqCst);
        })?;
    }

    let mut backend = WindowsBackend::new();

    let monitors = backend.list_monitors()?;
    println!("Monitors ({}):", monitors.len());
    for m in &monitors {
        let tag = if m.is_primary { " (primary)" } else { "" };
        println!(
            "  {}{tag}  bounds=({}, {}, {}, {})  scale={:.2}",
            m.id, m.bounds.left, m.bounds.top, m.bounds.right, m.bounds.bottom,
            m.scale_factor,
        );
    }

    let icons_snap = backend.list_icons()?;
    println!("\n{} desktop icons", icons_snap.len());
    if icons_snap.is_empty() {
        println!("Nothing to wiggle — bailing out.");
        return Ok(());
    }
    // Freeze the (id, current-position) pairs for the whole run — the
    // shell icons are never actually moved, so these stay valid.
    let icons: Vec<(IconId, Point)> =
        icons_snap.iter().map(|i| (i.id.clone(), i.position)).collect();

    println!(
        "\nLooping until Ctrl+C — shift range: {}..={} px right, {} icons per cycle.",
        SHIFT_PX_MIN, SHIFT_PX_MAX, icons.len(),
    );

    let mut rng = rand::rng();
    let mut cycle: u32 = 0;
    while !stop.load(Ordering::SeqCst) {
        cycle += 1;
        let shift_x = rng.random_range(SHIFT_PX_MIN..=SHIFT_PX_MAX);
        if let Err(e) = run_cycle(&mut backend, &icons, shift_x, cycle) {
            eprintln!("cycle {cycle} bailed: {e}");
            // Give the shell a beat to settle before retrying so a
            // transient failure (e.g. Explorer restart mid-cycle)
            // doesn't spin into a tight error loop.
            std::thread::sleep(StdDuration::from_millis(500));
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(HOLD_BETWEEN_CYCLES);
    }

    println!("\nStopping after {cycle} cycle(s). Icons should be back where they started.");
    Ok(())
}

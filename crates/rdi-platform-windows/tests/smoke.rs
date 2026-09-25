//! Smoke tests that talk to the **real** Windows desktop.
//!
//! These are ignored by default because they require:
//!
//! * Running on Windows (obviously — the whole crate is `#[cfg(windows)]`).
//! * An interactive desktop session (they will fail in CI runners that
//!   have no Explorer window).
//!
//! Run them explicitly with:
//!
//! ```powershell
//! cargo test -p rdi-platform-windows --test smoke -- --ignored --nocapture
//! ```
//!
//! Each test is deliberately **read-only** (the only exception restores
//! every icon to the exact position it started from) so running the
//! suite never leaves the user's desktop scrambled.

#![cfg(windows)]

use rdi_core::{AnimationOptions, DesktopBackend, DesktopController};
use rdi_platform_windows::WindowsBackend;

#[test]
#[ignore]
fn timeline_real_icon_visibility_restores_without_moving() {
    use rdi_core::{Curve, Duration, Effect, IconAnimationSpec, TimelineCloseMode, TimelineState};
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, FindWindowExW, IsWindowVisible};
    use windows::core::{BOOL, w};

    unsafe extern "system" fn find_listview(window: HWND, context: LPARAM) -> BOOL {
        // SAFETY: EnumWindows synchronously borrows the caller's output; names are static.
        unsafe {
            let output = &mut *(context.0 as *mut Option<HWND>);
            if let Ok(view) = FindWindowExW(Some(window), None, w!("SHELLDLL_DefView"), None) {
                if let Ok(list) = FindWindowExW(Some(view), None, w!("SysListView32"), None) {
                    *output = Some(list);
                }
            }
        }
        true.into()
    }
    let mut listview: Option<HWND> = None;
    // SAFETY: listview remains alive for the synchronous enumeration callback.
    unsafe { EnumWindows(Some(find_listview), LPARAM((&mut listview as *mut Option<HWND>) as isize)).unwrap(); }
    let listview = listview.expect("desktop ListView");
    let visible = || {
        // SAFETY: queries only the Shell HWND found by enumeration.
        unsafe { IsWindowVisible(listview).as_bool() }
    };
    assert!(visible(), "test requires initially visible real icons");
    let controller = DesktopController::new(WindowsBackend::new()).unwrap();
    let before = controller.list_icons().unwrap();
    assert!(!before.is_empty());
    let flags = controller.get_flags().unwrap();
    for with_shader in [false, true] {
        let program = with_shader.then(|| rdi_platform_windows::shader::compile(rdi_platform_windows::shader::BuiltinShader::Glitch).unwrap());
        let specs = before.iter().map(|icon| {
            let mut spec = IconAnimationSpec::new(icon.id.clone(), icon.position,
                Duration::fixed(std::time::Duration::from_secs(2)), Curve::linear());
            spec.effect = program.as_ref().map(|shader| Effect {
                shader: shader.clone(), params: [12.0, 4.0, 6.0, 30.0], padding_px: 24,
                envelope: Curve::linear(), seed: 0.0,
            });
            spec
        }).collect();
        let timeline = controller.prepare(specs, AnimationOptions::default()).unwrap().open_timeline().unwrap();
        assert!(!visible());
        assert!(!timeline.real_icons_visible());
        timeline.seek(0.5).unwrap();
        for shown in [true, false, true, false] {
            timeline.set_real_icons_visible(shown).unwrap();
            assert_eq!(visible(), shown);
            assert_eq!(controller.get_flags().unwrap() & 0x200 == 0, shown);
            assert_eq!(timeline.real_icons_visible(), shown);
            assert_eq!(timeline.position(), 0.5);
            assert_eq!(timeline.state(), TimelineState::Paused);
        }
        timeline.close(TimelineCloseMode::RestoreOrigins).unwrap();
        assert!(visible());
        assert_eq!(controller.get_flags().unwrap(), flags);
        let after = controller.list_icons().unwrap();
        for icon in &before {
            assert_eq!(after.iter().find(|entry| entry.id == icon.id).unwrap().position, icon.position);
        }
        println!("shader={with_shader}: real-icon visibility toggles and restoration verified on {} icons", before.len());
    }
}

#[test]
#[ignore]
fn desktop_info_and_grid_planning_are_read_only() {
    use rdi_core::{Curve, Duration, IconAnimationSpec, Point, resolve_grid_targets};
    let controller = DesktopController::new(WindowsBackend::new()).unwrap();
    let before = controller.list_icons().unwrap();
    let flags = controller.get_flags().unwrap();
    let info = controller.desktop_info().unwrap();
    println!("desktop metrics: {info:#?}");
    assert!(!info.monitors.is_empty());
    assert_eq!(info.monitors.len(), info.grids.len());
    for (monitor, grid) in info.monitors.iter().zip(&info.grids) {
        assert!(monitor.resolution().x > 0 && monitor.resolution().y > 0);
        assert!(monitor.dpi() > 0.0);
        assert!(grid.cell_size.x > 0 && grid.cell_size.y > 0);
        assert!(grid.icon_size.x > 0 && grid.icon_size.y > 0);
        if let Some(origin) = grid.origin {
            assert!(grid.work_area.contains(origin));
            assert!(before.iter().filter(|icon| monitor.bounds.contains(icon.position)).any(|icon| {
                (icon.position.x - origin.x).rem_euclid(grid.cell_size.x) == 0
                    && (icon.position.y - origin.y).rem_euclid(grid.cell_size.y) == 0
            }));
        }
    }
    if let Some(grid) = info.grids.iter().find(|grid| grid.origin.is_some() && grid.columns * grid.rows >= before.len() as u32) {
        let origin = grid.origin.unwrap();
        let mut specs: Vec<_> = before.iter().map(|icon| IconAnimationSpec::new(icon.id.clone(),
            Point::new(origin.x + 1, origin.y + 1), Duration::fixed(std::time::Duration::ZERO), Curve::linear())).collect();
        resolve_grid_targets(&mut specs, &before, &info).unwrap();
        for (index, spec) in specs.iter().enumerate() {
            assert_eq!((spec.target.x - origin.x).rem_euclid(grid.cell_size.x), 0);
            assert_eq!((spec.target.y - origin.y).rem_euclid(grid.cell_size.y), 0);
            assert!(specs[..index].iter().all(|previous| previous.target != spec.target));
        }
        println!("planned {} distinct destinations without moving icons", specs.len());
    }
    let after = controller.list_icons().unwrap();
    assert_eq!(controller.get_flags().unwrap(), flags);
    for icon in &before {
        assert_eq!(after.iter().find(|entry| entry.id == icon.id).unwrap().position, icon.position);
    }
}

#[test]
#[ignore]
fn list_icons_returns_something() {
    let mut backend = WindowsBackend::new();
    let icons = backend.list_icons().expect("list_icons failed");
    println!("Found {} desktop icons", icons.len());
    for icon in icons.iter().take(5) {
        println!(
            "  id={} name={:?} pos=({}, {}) virtual={}",
            icon.id.as_str(),
            icon.display_name,
            icon.position.x,
            icon.position.y,
            icon.is_virtual,
        );
    }
    // The exact count is machine-dependent, but a non-empty desktop
    // must have at least one item.
    assert!(!icons.is_empty(), "no icons on the desktop?");
}

#[test]
#[ignore]
fn read_flags_and_toggle_is_reversible() {
    let mut backend = WindowsBackend::new();
    let before = backend.get_flags().expect("get_flags failed");
    println!("initial flags = 0x{before:08x}");

    // FWF_SNAPTOGRID = 0x4 — the same bit the animation engine touches.
    const FWF_SNAPTOGRID: u32 = 0x4;
    backend
        .apply_flags(FWF_SNAPTOGRID, before ^ FWF_SNAPTOGRID)
        .expect("apply_flags toggle failed");
    let toggled = backend.get_flags().expect("get_flags mid failed");
    assert_ne!(before & FWF_SNAPTOGRID, toggled & FWF_SNAPTOGRID);

    // Restore.
    backend
        .apply_flags(FWF_SNAPTOGRID, before & FWF_SNAPTOGRID)
        .expect("apply_flags restore failed");
    let after = backend.get_flags().expect("get_flags end failed");
    assert_eq!(before & FWF_SNAPTOGRID, after & FWF_SNAPTOGRID);
}

#[test]
#[ignore]
fn controller_end_to_end_no_op_animation() {
    // Sanity-check that `DesktopController::new(WindowsBackend::new())`
    // reaches the tick loop and returns cleanly. Every icon animates
    // to *its current position*, so nothing visibly moves.
    let ctrl =
        DesktopController::new(WindowsBackend::new()).expect("controller construction failed");
    let icons = ctrl.list_icons().expect("list_icons failed");
    println!("controller sees {} icons", icons.len());

    if icons.is_empty() {
        println!("skipping animation: no icons on desktop");
        return;
    }

    // Build no-op specs (target == current position).
    use rdi_core::{Curve, Duration, IconAnimationSpec};
    use std::time::Duration as StdDuration;

    let specs: Vec<IconAnimationSpec> = icons
        .iter()
        .map(|s| {
            IconAnimationSpec::new(
                s.id.clone(),
                s.position,
                Duration::fixed(StdDuration::from_millis(50)),
                Curve::linear(),
            )
        })
        .collect();

    let handle = ctrl
        .animate(specs, AnimationOptions::default())
        .expect("animate failed");
    let reason = handle
        .wait_timeout(StdDuration::from_secs(5))
        .expect("animation timed out");
    println!("finish reason: {reason:?}");
}

#[test]
#[ignore]
fn list_monitors_reports_at_least_primary() {
    let mut backend = WindowsBackend::new();
    let monitors = backend.list_monitors().expect("list_monitors failed");
    println!("Found {} monitors", monitors.len());
    for m in &monitors {
        println!(
            "  id={:?} primary={} bounds=({},{},{},{}) work=({},{},{},{}) scale={:.2}",
            m.id,
            m.is_primary,
            m.bounds.left, m.bounds.top, m.bounds.right, m.bounds.bottom,
            m.work_area.left, m.work_area.top, m.work_area.right, m.work_area.bottom,
            m.scale_factor,
        );
    }
    assert!(!monitors.is_empty(), "no monitors reported");
    assert!(
        monitors.iter().any(|m| m.is_primary),
        "no monitor was flagged as primary"
    );
    // Every monitor must have positive dimensions.
    for m in &monitors {
        assert!(m.bounds.width() > 0);
        assert!(m.bounds.height() > 0);
        assert!(m.scale_factor > 0.0);
    }
}

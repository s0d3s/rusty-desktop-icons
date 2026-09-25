//! [`WindowsBackend`] — concrete [`DesktopBackend`] implementation.
//!
//! The backend lives on `rdi-core`'s worker thread and holds:
//!
//! * a [`ComContext`] with the cached `IFolderView2`,
//! * a [`HashMap<IconId, OwnedPidl>`] populated by `list_icons` and
//!   consulted by `set_positions`.
//!
//! On any COM error from a folder-view call the cached view is dropped
//! via [`ComContext::invalidate`] — the next call re-acquires it.
//! This mirrors the legacy `with_folder_view` retry helper.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration as StdDuration, Instant};

use rdi_core::{
    DesktopBackend, DesktopError, FinalCommitOutcome, IconId, IconLabel, IconRenderPlan,
    IconSnapshot, MonitorInfo, OverlayRenderOptions, Point, SnapshotFrame,
};
use windows::Win32::Foundation::{HWND, LPARAM, MAX_PATH, POINT, WPARAM};
use windows::Win32::UI::Controls::LVM_GETITEMSPACING;
use windows::Win32::UI::Shell::Common::{ITEMIDLIST, STRRET};
use windows::Win32::UI::Shell::{
    FOLDERVIEWMODE, IEnumIDList, IShellFolder, SHGDN_NORMAL, SHGetPathFromIDListW,
    SVGIO_ALLVIEW, SVSI_POSITIONITEM, StrRetToStrW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, IsWindowVisible, SendMessageTimeoutW, ShowWindow, SMTO_ABORTIFHUNG, SW_HIDE, SW_SHOW,
};
use windows::core::{PCWSTR, PWSTR, w};

use crate::com::ComContext;
use crate::ids::build_icon_id;
use crate::imaging::extract_icon_bitmap;
use crate::layout::{dpi_for_scale, IconLayout};
use crate::monitors::enumerate_monitors;
use crate::overlay::OverlaySession;
use crate::pidl::{CoTaskMemWStr, OwnedPidl, wcslen_bounded};
use crate::text::label_metrics_for_dpi;
use tracing::{debug, info, trace, warn};

/// `FWF_HIDEICONS` from `<shobjidl_core.h>` — set on the desktop
/// folder view to make Explorer stop drawing the real icons while
/// the overlay owns the visible pixels. Cleared before
/// `finalize_overlay_session` returns.
const FWF_HIDEICONS: u32 = 0x0000_0200;

/// How long to wait for `IFolderView2::GetItemPosition` to confirm
/// that the shell has moved at least one icon to its new position.
///
/// Explorer typically updates its internal layout within a few
/// milliseconds of `SelectAndPositionItems` returning; 250 ms is
/// generous enough that a slow SysListView32 doesn't leave the overlay
/// up until it becomes noticeable.
const SHELL_CONFIRM_TIMEOUT: StdDuration = StdDuration::from_millis(250);

/// Sleep between `GetItemPosition` polls while waiting for the shell
/// to confirm the final commit. 5 ms is generous — most confirmations
/// happen in one poll.
const SHELL_CONFIRM_POLL: StdDuration = StdDuration::from_millis(5);

/// How long the overlay stays visible AFTER restoring the real
/// icons but BEFORE being destroyed, so the shell + DWM composition
/// pipeline has a chance to push a fresh frame containing the real
/// icons at their final positions. Without this, the overlay's DComp
/// visual is torn down before DWM composites one frame of "shell has
/// updated" pixels, producing a single-frame blank flash at the
/// reveal.
///
/// **Empirical**: 50 ms is not enough — the blank flash is still
/// occasionally visible on a 250 % Windows 11 desktop. 300 ms gives a
/// clean reveal. Callers with faster hardware could reasonably want
/// this as a tunable option.
const SHELL_REVEAL_SETTLE: StdDuration = StdDuration::from_millis(300);

/// Windows-native `DesktopBackend` driving Explorer's desktop folder view.
pub struct WindowsBackend {
    com: ComContext,
    /// PIDLs cached from the most recent `list_icons` call. The engine
    /// calls `list_icons` at the start of every animation, so the
    /// animation's ticks reuse these instead of re-enumerating the
    /// desktop.
    pidl_cache: HashMap<IconId, OwnedPidl>,
    /// Display names cached alongside `pidl_cache`. Populated by the
    /// same enumeration pass; used by
    /// [`Self::enrich_plans`](WindowsBackend::enrich_plans) to build
    /// `IconLabel`s without a second Shell round-trip.
    display_name_cache: HashMap<IconId, String>,
    overlay: OverlayState,
}

/// The overlay animation lifecycle, as a state machine.
///
/// Transitions are one-way and each is owned by exactly one method:
/// `begin_overlay_session` builds `Prepared`, the *first*
/// `commit_overlay_frame` promotes it to `Live`, and
/// `finalize_overlay_session` (or `Drop`) consumes it back to `Idle`.
enum OverlayState {
    /// No animation in flight. The real desktop icons are visible and
    /// owned entirely by Explorer.
    Idle,
    /// Session built and the first frame pre-rendered, but the overlay
    /// window is still hidden and the Shell has not been touched — the
    /// real icons are visible at their *source* positions.
    Prepared {
        session: OverlaySession,
        /// Icon targets to hand the Shell on the first commit.
        pending_final_positions: Vec<(IconId, Point)>,
    },
    /// Overlay visible; the real icons have been teleported to their
    /// final positions and hidden. Holding a [`ShellHideGuard`] here is
    /// what makes "the icons always come back" a property of the type
    /// rather than of every error path.
    Live {
        session: OverlaySession,
        hide: ShellHideGuard,
        /// Non-empty only when the first-frame teleport failed and
        /// `finalize_overlay_session` still has to retry it.
        pending_final_positions: Vec<(IconId, Point)>,
    },
}

/// Owns the two Shell-side actions that hide the real desktop icons
/// for the duration of an overlay animation.
///
/// Restoring is split because the two halves have different
/// requirements: `ShowWindow(SW_SHOW)` needs nothing but the HWND, so
/// `Drop` can always do it, while clearing `FWF_HIDEICONS` needs
/// `IFolderView2` and therefore has to go through
/// [`WindowsBackend::restore_real_icons`].
///
/// The two invariants this enforces at the type level: `FWF_HIDEICONS`
/// is always cleared again, and the `SysListView32` is always reshown.
struct ShellHideGuard {
    /// The `SysListView32` that was hidden, if one was found.
    hidden_syslistview: Option<HWND>,
    /// Whether `FWF_HIDEICONS` was set by this guard. Stays `true`
    /// if clearing it fails, so the `Drop` backstop can see it.
    flag_set: bool,
}

impl ShellHideGuard {
    /// Restore `SysListView32` visibility. Idempotent.
    fn show_syslistview(&mut self) {
        if let Some(hwnd) = self.hidden_syslistview.take() {
            // SAFETY: `ShowWindow` is safe on any HWND value.
            let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };
        }
    }
}

impl Drop for ShellHideGuard {
    fn drop(&mut self) {
        self.show_syslistview();
        if self.flag_set {
            warn!(
                "ShellHideGuard dropped with FWF_HIDEICONS still set — the next \
                 begin_overlay_session will clear it"
            );
        }
    }
}

// SAFETY: `IFolderView2` (and every COM interface it transitively holds)
// is apartment-threaded, not `Sync`. `WindowsBackend` is owned
// exclusively by the engine's dedicated worker thread — every method
// call on the trait happens on the same thread that constructed it.
// `Send` is asserted so `DesktopController::new` can move the backend
// onto that worker thread. The type is deliberately NOT `Sync`: no
// path in `rdi-core` shares a `&Backend` across threads.
unsafe impl Send for WindowsBackend {}

impl Drop for WindowsBackend {
    /// Best-effort cleanup for the "worker thread panicked mid-
    /// animation" case. Dropping while the overlay is still `Live`
    /// means the normal `finalize_overlay_session` path was skipped;
    /// mirroring it here keeps the desktop from being left with
    /// hidden icons after a panic.
    ///
    /// The body is wrapped in `catch_unwind` because this runs during
    /// unwinding: `restore_real_icons` makes a COM call, and a panic
    /// inside a `Drop` that is itself unwinding aborts the process,
    /// skipping the very cleanup this impl exists to perform.
    ///
    /// This does **not** cover `abort()` or `TerminateProcess`,
    /// which don't run `Drop`. Startup cleanup in
    /// `begin_overlay_session` handles those.
    fn drop(&mut self) {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // Taking the state first means a panic below still leaves
            // `self.overlay` in a coherent `Idle`.
            if let OverlayState::Live { session, hide, .. } =
                std::mem::replace(&mut self.overlay, OverlayState::Idle)
            {
                self.restore_real_icons(hide);
                // Explicit: the DComp visual must be gone before the
                // COM interfaces in `self.com` are released (Rust
                // drops fields in declaration order).
                drop(session);
            }
        }));
        if outcome.is_err() {
            // Deliberately not `warn!`: the desktop is visibly broken
            // at this point.
            tracing::error!(
                "panic while restoring desktop state during Drop; desktop icons \
                 may remain hidden until the next animation"
            );
        }
    }
}

impl WindowsBackend {
    /// Construct a new backend. COM is initialised lazily — no work is
    /// done until the first trait method call. Never fails; errors
    /// surface at that first call instead.
    ///
    /// Also opts the current process into Per-Monitor V2 DPI
    /// awareness (idempotent; failure is silently accepted for
    /// processes whose awareness was already fixed by manifest).
    /// This MUST run before any DPI-sensitive Shell call — in
    /// particular before [`Self::list_monitors`] and
    /// [`Self::begin_overlay_session`] — otherwise
    /// `GetDpiForMonitor` and `SPI_GETICONMETRICS` return virtualised
    /// 96-DPI values and the overlay renders icons + labels at
    /// ~1/scale of their real physical size on high-DPI displays.
    pub fn new() -> Self {
        crate::overlay::ensure_dpi_awareness();
        Self {
            com: ComContext::new(),
            pidl_cache: HashMap::new(),
            display_name_cache: HashMap::new(),
            overlay: OverlayState::Idle,
        }
    }

    /// Refresh [`Self::pidl_cache`] by walking `IEnumIDList` and storing
    /// every (id, pidl) pair. Returns the collected [`IconSnapshot`]s.
    fn enumerate(&mut self) -> Result<Vec<IconSnapshot>, DesktopError> {
        let view = self.com.folder_view()?;

        let result = (|| -> Result<Vec<IconSnapshot>, DesktopError> {
            // SAFETY: the folder view was just acquired successfully;
            // its v-table is valid. The two calls below either yield
            // valid interfaces or a propagated error.
            let (folder, enumerator) = unsafe {
                let folder: IShellFolder = view.GetFolder().map_err(|e| {
                    DesktopError::BackendUnavailable(format!(
                        "IFolderView2::GetFolder failed: {e}"
                    ))
                })?;
                let enumerator: IEnumIDList = view.Items(SVGIO_ALLVIEW).map_err(|e| {
                    DesktopError::BackendUnavailable(format!(
                        "IFolderView2::Items(SVGIO_ALLVIEW) failed: {e}"
                    ))
                })?;
                (folder, enumerator)
            };

            let mut snapshots = Vec::new();
            let mut fresh_cache = HashMap::new();
            let mut fresh_names: HashMap<IconId, String> = HashMap::new();

            loop {
                let mut pidl_raw: *mut ITEMIDLIST = std::ptr::null_mut();
                let mut fetched: u32 = 0;
                // SAFETY: `IEnumIDList::Next` writes exactly one PIDL
                // pointer into the stack slot when it returns S_OK
                // with `fetched == 1`.
                let hr = unsafe {
                    enumerator.Next(
                        std::slice::from_mut(&mut pidl_raw),
                        Some(&mut fetched as *mut u32),
                    )
                };
                if hr.is_err() || fetched == 0 || pidl_raw.is_null() {
                    break;
                }
                let pidl = OwnedPidl::from_raw(pidl_raw);

                // SAFETY: `pidl` was just returned by the shell; the
                // helpers below only read from it.
                let (path_utf16, display_utf16, position) = unsafe {
                    let path = path_of(pidl.as_ptr());
                    let display = display_name_of(&folder, pidl.as_ptr())?;
                    let pt = view.GetItemPosition(pidl.as_ptr()).map_err(|e| {
                        DesktopError::BackendUnavailable(format!(
                            "IFolderView2::GetItemPosition failed: name={:?}, \
                             path={:?}, HRESULT=0x{:08X}: {}",
                            String::from_utf16_lossy(&display),
                            if path.is_empty() {
                                "<unavailable>".to_owned()
                            } else {
                                String::from_utf16_lossy(&path)
                            },
                            e.code().0 as u32,
                            e.message(),
                        ))
                    })?;
                    (path, display, pt)
                };

                let is_virtual = path_utf16.is_empty();
                let id = build_icon_id(&path_utf16, &display_utf16);
                let display_string = String::from_utf16_lossy(&display_utf16);

                let path_pb = if is_virtual {
                    None
                } else {
                    Some(PathBuf::from(String::from_utf16_lossy(&path_utf16)))
                };

                snapshots.push(IconSnapshot::new(
                    id.clone(),
                    display_string.clone(),
                    path_pb,
                    is_virtual,
                    Point::new(position.x, position.y),
                ));
                // Last write wins if two icons hash to the same id — in
                // practice that never happens because shell paths are
                // unique and virtual display names are unique too.
                fresh_names.insert(id.clone(), display_string);
                fresh_cache.insert(id, pidl);
            }

            self.pidl_cache = fresh_cache;
            self.display_name_cache = fresh_names;
            Ok(snapshots)
        })();

        if result.is_err() {
            // A folder-view failure invalidates the cached interface.
            self.com.invalidate();
            self.pidl_cache.clear();
            self.display_name_cache.clear();
        }
        result
    }
}

impl Default for WindowsBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopBackend for WindowsBackend {
    fn list_icons(&mut self) -> Result<Vec<IconSnapshot>, DesktopError> {
        self.enumerate()
    }

    fn get_flags(&mut self) -> Result<u32, DesktopError> {
        let view = self.com.folder_view()?;
        // SAFETY: `view` is a valid `IFolderView2` interface pointer.
        let result = unsafe { view.GetCurrentFolderFlags() };
        match result {
            Ok(flags) => Ok(flags),
            Err(e) => {
                self.com.invalidate();
                Err(DesktopError::BackendUnavailable(format!(
                    "IFolderView2::GetCurrentFolderFlags failed: {e}"
                )))
            }
        }
    }

    fn apply_flags(&mut self, mask: u32, values: u32) -> Result<(), DesktopError> {
        let view = self.com.folder_view()?;
        // SAFETY: `view` is a valid `IFolderView2` interface pointer.
        // `SetCurrentFolderFlags(mask, values)` applies the same
        // masked-update semantics documented on the trait
        // (`new = (old & !mask) | (values & mask)`), so no translation
        // is needed.
        let result = unsafe { view.SetCurrentFolderFlags(mask, values) };
        match result {
            Ok(()) => Ok(()),
            Err(e) => {
                self.com.invalidate();
                Err(DesktopError::BackendUnavailable(format!(
                    "IFolderView2::SetCurrentFolderFlags failed: {e}"
                )))
            }
        }
    }

    fn set_positions(
        &mut self,
        moves: &[(IconId, Point)],
    ) -> Result<Vec<IconId>, DesktopError> {
        if moves.is_empty() {
            return Ok(Vec::new());
        }

        // If any id in the batch isn't in the cache, refresh — this
        // covers the "direct positioning without a preceding
        // list_icons" flow. `enumerate` also refreshes the folder view
        // if needed.
        let has_uncached = moves.iter().any(|(id, _)| !self.pidl_cache.contains_key(id));
        if has_uncached {
            let _ = self.enumerate()?;
        }

        let view = self.com.folder_view()?;

        // Partition into (resolvable, missing).
        let mut pidl_ptrs: Vec<*const ITEMIDLIST> = Vec::with_capacity(moves.len());
        let mut points: Vec<POINT> = Vec::with_capacity(moves.len());
        let mut missing: Vec<IconId> = Vec::new();

        for (id, pt) in moves {
            match self.pidl_cache.get(id) {
                Some(pidl) => {
                    pidl_ptrs.push(pidl.as_ptr());
                    points.push(POINT { x: pt.x, y: pt.y });
                }
                None => missing.push(id.clone()),
            }
        }

        if pidl_ptrs.is_empty() {
            return Ok(missing);
        }

        // SAFETY: `pidl_ptrs` and `points` are both `pidl_ptrs.len()`
        // elements long and outlive the call. Every `*const ITEMIDLIST`
        // was obtained from `IEnumIDList::Next` on this backend and is
        // still owned by `self.pidl_cache`.
        let hr = unsafe {
            view.SelectAndPositionItems(
                pidl_ptrs.len() as u32,
                pidl_ptrs.as_ptr(),
                Some(points.as_ptr()),
                SVSI_POSITIONITEM.0 as u32,
            )
        };
        if let Err(e) = hr {
            self.com.invalidate();
            self.pidl_cache.clear();
            self.display_name_cache.clear();
            return Err(DesktopError::BackendUnavailable(format!(
                "IFolderView2::SelectAndPositionItems failed: {e}"
            )));
        }

        // Explorer's `SysListView32` accepts the new item positions
        // but defers painting them — its message loop batches
        // invalidations and only flushes them when it goes idle, which
        // never happens while frames are committed back-to-back. The
        // synchronous repaint is what keeps the desktop from looking
        // frozen until the tick loop ends. See
        // `ComContext::redraw_folder_view`.
        self.com.redraw_folder_view();

        Ok(missing)
    }

    fn list_monitors(&mut self) -> Result<Vec<MonitorInfo>, DesktopError> {
        // Defensive: `enumerate_monitors` uses `GetDpiForMonitor`,
        // which returns virtualised 96 DPI if the process is DPI
        // unaware. `WindowsBackend::new` already opts into PMv2, but
        // callers that construct the trait object via other paths
        // (mocks, tests, `mem::replace`) may bypass that constructor.
        // The call is idempotent.
        crate::overlay::ensure_dpi_awareness();
        // Pure GDI enumeration — does not touch `IFolderView2`, so
        // there is nothing to invalidate on failure.
        enumerate_monitors()
    }

    fn desktop_info(&mut self, icons: &[IconSnapshot]) -> Result<rdi_core::DesktopInfo, DesktopError> {
        let monitors = self.list_monitors()?;
        let view = self.com.folder_view()?;
        let mut mode = FOLDERVIEWMODE(0);
        let mut logical_size = 0;
        // SAFETY: the STA-owned view is valid and both output pointers are stack locals.
        if let Err(error) = unsafe { view.GetViewModeAndIconSize(&mut mode, &mut logical_size) } {
            self.com.invalidate();
            return Err(DesktopError::BackendUnavailable(format!("icon size query failed: {error}")));
        }
        if logical_size <= 0 {
            return Err(DesktopError::BackendUnavailable("invalid Shell icon size".into()));
        }
        let list = self.find_syslistview32().ok_or_else(||
            DesktopError::BackendUnavailable("desktop ListView is unavailable".into()))?;
        let mut packed = 0usize;
        // SAFETY: this pointer-free message returns packed dimensions; the output
        // is written by user32 locally, never dereferenced by the Explorer process.
        let result = unsafe { SendMessageTimeoutW(list, LVM_GETITEMSPACING, WPARAM(0),
            LPARAM(0), SMTO_ABORTIFHUNG, 1000, Some(&mut packed)) };
        let spacing = Point::new((packed & 0xffff) as i32, ((packed >> 16) & 0xffff) as i32);
        if result.0 == 0 || spacing.x <= 0 || spacing.y <= 0 {
            self.com.invalidate();
            return Err(DesktopError::BackendUnavailable("live desktop grid spacing is unavailable".into()));
        }
        let mut grids = Vec::with_capacity(monitors.len());
        for monitor in &monitors {
            let positions: Vec<_> = icons.iter().filter(|icon| monitor.bounds.contains(icon.position))
                .map(|icon| icon.position).collect();
            let origin = rdi_core::IconGrid::infer_origin(monitor.work_area, spacing, &positions);
            let metrics = label_metrics_for_dpi(dpi_for_scale(monitor.scale_factor));
            let layout = IconLayout::resolve((logical_size as u32, logical_size as u32),
                monitor.scale_factor, &metrics);
            grids.push(rdi_core::IconGrid::new(monitor.id.clone(), monitor.work_area,
                Point::new(layout.size_px.0 as i32, layout.size_px.1 as i32), spacing,
                origin, true)?);
        }
        let bounds = monitors.iter().map(|monitor| monitor.bounds).reduce(|left, right| {
            rdi_core::Rect::new(left.left.min(right.left), left.top.min(right.top),
                left.right.max(right.right), left.bottom.max(right.bottom))
        }).ok_or_else(|| DesktopError::BackendUnavailable("no desktop monitors".into()))?;
        Ok(rdi_core::DesktopInfo { bounds, monitors, grids })
    }

    fn begin_overlay_session(
        &mut self,
        plans: &[IconRenderPlan],
        render_options: OverlayRenderOptions,
    ) -> Result<(), DesktopError> {
        if !matches!(self.overlay, OverlayState::Idle) {
            return Err(DesktopError::OverlayUnavailable(
                "an overlay session is already open on this backend".into(),
            ));
        }

        // Stale-state recovery: `FWF_HIDEICONS` already set when a new
        // session opens must be a leftover from a previous run that
        // crashed before it could clean up. Legitimate callers never
        // set this flag on the desktop folder view, so clearing it is
        // safe.
        if let Ok(current) = self.get_flags() {
            if current & FWF_HIDEICONS != 0 {
                warn!(
                    flags = format_args!("0x{current:08x}"),
                    "found stale FWF_HIDEICONS at overlay session start — assuming \
                     a prior session crashed; clearing"
                );
                let _ = self.apply_flags(FWF_HIDEICONS, 0);
            }
        }

        // Enrich the engine-supplied plans with Shell-sourced
        // metadata (real icon bitmap, label, refined size, selection
        // state) before handing them to the overlay renderer.
        // Failures here degrade gracefully: bitmap missing → placeholder;
        // label missing → no text; selection unknown → drawn as
        // unselected.
        //
        // `render_options` gates the individual decoration passes
        // (labels, shortcut arrow overlay, admin shield overlay).
        // A disabled decoration skips both its rendering and its
        // Shell COM probe — see
        // [`extract_icon_bitmap`](crate::imaging::extract_icon_bitmap)
        // and the `if render_options.draw_labels` check in
        // `enrich_plans`.
        let enriched = self.enrich_plans(plans, render_options);

        // Cache the icon → final-position map so the first
        // `commit_overlay_frame` can teleport the real Shell icons
        // to their targets before hiding them.
        let pending_final_positions = enriched
            .iter()
            .filter(|plan| !plan.stationary)
            .map(|p| (p.id.clone(), p.final_position))
            .collect();

        // Build the overlay window + Direct2D + DirectComposition
        // resources; render the first frame at each icon's source
        // position while the window is still hidden.
        let session = OverlaySession::new(&enriched)?;
        self.overlay = OverlayState::Prepared {
            session,
            pending_final_positions,
        };
        Ok(())
    }

    fn commit_overlay_frame(
        &mut self,
        positions: &[(IconId, Point)],
    ) -> Result<(), DesktopError> {
        // ---- Cancellation watchdog -------------------------
        //
        // Pump any queued messages so the overlay WNDPROC has a
        // chance to observe broadcast messages
        // (`TaskbarCreated`, `WM_DISPLAYCHANGE`, `WM_DPICHANGED`),
        // then read-and-clear the cancel atom. Also cross-check the
        // cached shell view HWND via `IsWindow` for the case where
        // the shell view silently vanishes without emitting
        // `TaskbarCreated`.
        //
        // A `DesktopError::OverlayCancelled` here is caught by the
        // engine's tick loop and results in a graceful
        // `FinishReason::Stopped(TeleportToTarget)` — the real icons
        // are already at their final positions from the first-frame
        // teleport, so this outcome is correct.
        self.poll_overlay_session()?;

        // Render + present. Scoped so the `session` borrow ends before
        // the Shell calls below, which need `&mut self`.
        match &mut self.overlay {
            OverlayState::Idle => {
                return Err(DesktopError::OverlayUnavailable(
                    "commit_overlay_frame called with no active overlay session".into(),
                ));
            }
            OverlayState::Prepared { session, .. } | OverlayState::Live { session, .. } => {
                // `trace!`: this runs once per tick (100 Hz default).
                trace!(icons = positions.len(), "overlay frame");
                session.commit_frame(positions)?;
            }
        }

        // First-frame housekeeping — done AFTER the overlay is
        // guaranteed to have presented at least one frame. This is the
        // one and only `Prepared → Live` transition:
        //   1. Teleport the real Shell icons to their final positions
        //      (they'll be behind the overlay for the rest of the
        //      animation).
        //   2. Hide the Shell's own drawing of them, and take
        //      ownership of undoing that via `ShellHideGuard`.
        let previous = std::mem::replace(&mut self.overlay, OverlayState::Idle);
        self.overlay = match previous {
            OverlayState::Prepared {
                session,
                pending_final_positions,
            } => {
                let pending = self.teleport_real_icons(pending_final_positions);
                let hide = self.hide_real_icons();
                OverlayState::Live {
                    session,
                    hide,
                    pending_final_positions: pending,
                }
            }
            already_live => already_live,
        };
        Ok(())
    }

    fn commit_visual_frame(&mut self, frame: &[rdi_core::IconFrame]) -> Result<(), DesktopError> {
        match &mut self.overlay {
            OverlayState::Prepared { session, .. } | OverlayState::Live { session, .. } => session.set_visual_frame(frame),
            OverlayState::Idle => {},
        }
        let positions: Vec<_> = frame.iter().map(|entry| (entry.id.clone(), entry.position)).collect();
        self.commit_overlay_frame(&positions)
    }

    fn discard_overlay_session(&mut self) {
        if matches!(self.overlay, OverlayState::Prepared { .. }) {
            self.overlay = OverlayState::Idle;
        }
    }

    fn validate_prepared_session(&mut self) -> Result<(), DesktopError> {
        self.poll_overlay_session()
    }

    fn set_real_icons_visible(&mut self, visible: bool) -> Result<(), DesktopError> {
        if !matches!(self.overlay, OverlayState::Live { .. }) {
            return Err(DesktopError::BackendUnavailable("real-icon visibility requires a live overlay".into()));
        }
        let hwnd = self.find_syslistview32().ok_or_else(||
            DesktopError::BackendUnavailable("desktop ListView is unavailable".into()))?;
        self.apply_flags(FWF_HIDEICONS, if visible { 0 } else { FWF_HIDEICONS })?;
        if let OverlayState::Live { hide, .. } = &mut self.overlay {
            hide.flag_set = true;
            hide.hidden_syslistview = Some(hwnd);
        }
        self.com.redraw_folder_view();
        // SAFETY: hwnd is the current Shell ListView; visibility calls borrow no memory.
        unsafe { let _ = ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE }); }
        // SAFETY: IsWindowVisible only queries the current Shell ListView handle.
        let window_visible = unsafe { IsWindowVisible(hwnd).as_bool() };
        if window_visible != visible {
            return Err(DesktopError::BackendUnavailable("desktop ListView visibility did not change as requested".into()));
        }
        if (self.get_flags()? & FWF_HIDEICONS == 0) != visible {
            return Err(DesktopError::BackendUnavailable("desktop icon flag did not change as requested".into()));
        }
        if visible {
            if let OverlayState::Live { hide, .. } = &mut self.overlay {
                hide.hidden_syslistview = None;
                hide.flag_set = false;
            }
        }
        Ok(())
    }

    fn poll_overlay_session(&mut self) -> Result<(), DesktopError> {
        crate::overlay::pump_pending_messages();
        if let Some(reason) = crate::overlay::take_cancel_reason() {
            return Err(DesktopError::OverlayCancelled(reason.description().into()));
        }
        if !self.com.is_shell_view_valid() {
            crate::overlay::signal_cancel(crate::overlay::CancelReason::ShellViewLost);
            let reason = crate::overlay::take_cancel_reason()
                .unwrap_or(crate::overlay::CancelReason::ShellViewLost);
            return Err(DesktopError::OverlayCancelled(reason.description().into()));
        }
        Ok(())
    }

    fn finalize_overlay_session(
        &mut self,
        final_positions: &[(IconId, Point)],
    ) -> Result<FinalCommitOutcome, DesktopError> {
        // Take the whole state up front: every path below leaves the
        // backend `Idle`, including the ones that bail early.
        let (mut session, hide, pending) =
            match std::mem::replace(&mut self.overlay, OverlayState::Idle) {
                OverlayState::Idle => (None, None, Vec::new()),
                OverlayState::Prepared {
                    session,
                    ..
                } => {
                    drop(session);
                    return Ok(FinalCommitOutcome::default());
                }
                OverlayState::Live {
                    session,
                    hide,
                    pending_final_positions,
                } => (Some(session), Some(hide), pending_final_positions),
            };

        // The engine's `final_positions` is the authoritative end state
        // and is NOT always what the first-frame teleport applied: a
        // `StopMode::LeaveInPlace` stop leaves icons mid-flight, so
        // committing only `pending` would snap them to their targets.
        let mut commit: Vec<(IconId, Point)> = final_positions.to_vec();
        // `pending` is non-empty only when the first-frame teleport
        // failed. Ids the engine no longer tracks still need the retry.
        for (id, pt) in pending {
            if !commit.iter().any(|(existing, _)| existing == &id) {
                commit.push((id, pt));
            }
        }

        let missing = if commit.is_empty() {
            Vec::new()
        } else {
            self.set_positions(&commit).unwrap_or_else(|e| {
                warn!(
                    error = %e,
                    icons = commit.len(),
                    "finalize: final set_positions failed; continuing with cleanup"
                );
                Vec::new()
            })
        };

        // Poll GetItemPosition to confirm the Shell has settled at
        // the final positions. Cheap on the happy path — the commit
        // above is usually a no-op repeat of the first-frame teleport,
        // so confirmation lands on the very first poll.
        let moved = self.await_shell_confirmation(final_positions, &missing);

        if let Some(session) = &mut session {
            let _ = session.clean_frame(final_positions);
        }
        if let Some(hide) = hide {
            self.restore_real_icons(hide);
        }

        // `RedrawWindow` on a cross-process HWND posts messages
        // synchronously via `SendMessage`, but DWM still needs one
        // composition frame to push the shell's fresh pixels to the
        // display — the overlay must stay on screen for that frame or
        // a single blank frame shows between overlay removal and shell
        // paint.
        self.com.redraw_folder_view();
        std::thread::sleep(SHELL_REVEAL_SETTLE);

        // Destroy the overlay window last so the real icons have a
        // chance to paint before the overlay disappears.
        drop(session);

        info!(
            moved = moved.len(),
            missing = missing.len(),
            "overlay session finalized"
        );
        Ok(FinalCommitOutcome {
            moved_ids: moved,
            missing_ids: missing,
        })
    }

    fn prepare_scene_renderer(&mut self, canvas: rdi_core::Canvas, plans: &[IconRenderPlan], options: OverlayRenderOptions) -> Result<Box<dyn rdi_core::SceneRenderer>, DesktopError> {
        canvas.validate()?;
        let positions: Vec<_> = plans.iter().map(|plan| (plan.id.clone(), plan.source_position)).collect();
        let mut enriched = self.build_snapshot_plans_with_size(canvas.dpi_scale, &positions, options, Some(canvas.icon_size));
        for (plan, source) in enriched.iter_mut().zip(plans) {
            plan.effect = source.effect.clone();
            plan.final_position = source.final_position;
            plan.stationary = source.stationary;
            if plan.image.is_none() {
                return Err(DesktopError::BackendUnavailable(format!("artwork unavailable for {}", plan.id.as_str())));
            }
        }
        Ok(Box::new(crate::overlay::SceneRenderer::new(canvas, &enriched)?))
    }

    fn capture_overlay(&mut self, seconds: f64) -> Result<rdi_core::CapturedFrame, DesktopError> {
        match &mut self.overlay {
            OverlayState::Live { session, .. } => session.capture(seconds),
            _ => Err(DesktopError::BackendUnavailable("no live overlay".into())),
        }
    }

    fn render_overlay_snapshot(
        &mut self,
        width_px: u32,
        height_px: u32,
        dpi_scale: f32,
        positions: &[(IconId, Point)],
        render_options: OverlayRenderOptions,
    ) -> Result<SnapshotFrame, DesktopError> {
        // The caller-supplied `dpi_scale` replaces the live
        // per-monitor scale factor, which is what makes a snapshot
        // resolution- and DPI-independent. Positions are already
        // overlay-local, and a snapshot is a single frame, so
        // `source_position == final_position`.
        let plans = self.build_snapshot_plans(dpi_scale, positions, render_options);
        let width = width_px.max(1);
        let height = height_px.max(1);
        let (pixels, geometry) = crate::overlay::render_snapshot(
            width, height, dpi_scale, &plans, positions,
        )?;
        Ok(SnapshotFrame {
            width,
            height,
            pixels,
            geometry,
        })
    }
}

// ---------------------------------------------------------------------------
// Shell helpers — used only from the `unsafe` blocks above.
// ---------------------------------------------------------------------------

/// Get the shell display name of `pidl` as UTF-16 (no trailing NUL).
///
/// # Safety
/// * `folder` must be the `IShellFolder` that owns `pidl`.
/// * `pidl` must be a valid PIDL relative to `folder`.
unsafe fn display_name_of(
    folder: &IShellFolder,
    pidl: *const ITEMIDLIST,
) -> Result<Vec<u16>, DesktopError> {
    // SAFETY forwarded to the caller — see the doc comment above. The
    // individual `unsafe {}` blocks below only exist because
    // `unsafe_op_in_unsafe_fn` is a hard error under Edition 2024.
    let mut strret: STRRET = unsafe { std::mem::zeroed() };
    unsafe {
        folder
            .GetDisplayNameOf(pidl, SHGDN_NORMAL, &mut strret)
            .map_err(|e| {
                DesktopError::BackendUnavailable(format!(
                    "IShellFolder::GetDisplayNameOf failed: {e}"
                ))
            })?;
    }
    let mut out_ptr = PWSTR::null();
    unsafe {
        StrRetToStrW(&mut strret, Some(pidl), &mut out_ptr).map_err(|e| {
            DesktopError::BackendUnavailable(format!("StrRetToStrW failed: {e}"))
        })?;
    }
    let owned = CoTaskMemWStr::from_raw(out_ptr);
    Ok(owned.to_vec())
}

/// Get the filesystem path of `pidl` as UTF-16 (no trailing NUL).
/// Returns an empty vec for virtual items (matching the legacy behaviour
/// when `SHGetPathFromIDListW` fails or returns an empty string).
///
/// # Safety
/// `pidl` must be a valid absolute PIDL.
unsafe fn path_of(pidl: *const ITEMIDLIST) -> Vec<u16> {
    let mut buf = [0u16; MAX_PATH as usize];
    // SAFETY: forwarded to the caller — `pidl` is a valid absolute PIDL.
    let ok = unsafe { SHGetPathFromIDListW(pidl, &mut buf) };
    if !ok.as_bool() {
        return Vec::new();
    }
    let len = wcslen_bounded(&buf, buf.len());
    buf[..len].to_vec()
}

// ---------------------------------------------------------------------------
// Overlay-session support methods on WindowsBackend.
// ---------------------------------------------------------------------------

impl WindowsBackend {
    /// Enrich a slice of engine-supplied `IconRenderPlan`s with Shell
    /// data before handing them to `OverlaySession`.
    ///
    /// * `size_px` — refined via [`Self::query_icon_size_px`].
    /// * `image` — filled via [`extract_icon_bitmap`] on the cached
    ///   PIDL; `None` if extraction fails (renderer falls back to a
    ///   solid placeholder).
    /// * `label` — pulled from `display_name_cache` (populated by the
    ///   most recent `list_icons`).
    ///
    /// The whole method is best-effort: any per-icon step that fails
    /// leaves the corresponding field at its default and the animation
    /// still runs.
    fn enrich_plans(
        &mut self,
        plans: &[IconRenderPlan],
        render_options: OverlayRenderOptions,
    ) -> Vec<IconRenderPlan> {
        let base_size_dip = self.query_icon_size_px().unwrap_or((48, 48));

        // Per-monitor DPI. `IFolderView2::GetViewModeAndIconSize`
        // reports in Explorer's DPI-awareness context, so crossing COM
        // into this PMv2 process it arrives at the 96-DPI baseline; the
        // scale factor is what turns it back into physical pixels. See
        // [`IconLayout::resolve`](crate::layout::IconLayout::resolve)
        // for the full rationale.
        let monitors = self.list_monitors().unwrap_or_default();

        plans
            .iter()
            .map(|plan| {
                let mut enriched = plan.clone();

                // Icons that fall outside every reported monitor
                // (extremely rare — e.g. an icon at (-99999, 0)) fall
                // back to the primary's DPI so they still render at
                // *some* plausible size.
                let icon_scale = monitors
                    .iter()
                    .find(|m| m.bounds.contains(plan.source_position))
                    .or_else(|| monitors.iter().find(|m| m.is_primary))
                    .map(|m| m.scale_factor.max(0.01))
                    .unwrap_or(1.0);

                // Metrics at the icon's own monitor DPI, so wrap sizing
                // tracks Explorer's paint there without a secondary
                // rescale.
                let metrics = label_metrics_for_dpi(dpi_for_scale(icon_scale));
                let layout = IconLayout::resolve(base_size_dip, icon_scale, &metrics);

                enriched.size_px = layout.size_px;
                enriched.render_offset_px = layout.render_offset_px;

                // Icon bitmap + optional shortcut-arrow overlay —
                // request the base at the per-icon rendered size so
                // `IShellItemImageFactory` returns a bitmap sharp at
                // that scale (SIIGBF_BIGGERSIZEOK lets the Shell
                // round up to the nearest available image). The
                // arrow is kept as a separate bitmap so the overlay
                // renderer can anchor it to the icon slot.
                let extracted = self.pidl_cache.get(&plan.id).and_then(|pidl| unsafe {
                    extract_icon_bitmap(
                        pidl.as_ptr(),
                        enriched.size_px.0,
                        render_options.draw_shortcut_overlay,
                        render_options.draw_shield_overlay,
                    )
                });
                if let Some(bundle) = extracted {
                    enriched.image = Some(bundle.base);
                    enriched.shortcut_arrow_image = bundle.shortcut_arrow;
                } else {
                    enriched.image = None;
                    enriched.shortcut_arrow_image = None;
                }

                enriched.label = if render_options.draw_labels {
                    self.display_name_cache
                        .get(&plan.id)
                        .cloned()
                        .map(|text| IconLabel {
                            text,
                            bounds_px: layout.label_bounds_px,
                        })
                } else {
                    None
                };

                // `final_position` was set by the engine when it built
                // the placeholder plan; pass it through unchanged so
                // `begin_overlay_session` can cache it for the
                // first-commit teleport.
                enriched.final_position = plan.final_position;

                enriched
            })
            .collect()
    }

    /// Build enriched `IconRenderPlan`s for the off-screen snapshot
    /// path. Mirrors [`Self::enrich_plans`] but uses a caller-supplied
    /// `dpi_scale` uniformly instead of resolving each icon's
    /// per-monitor scale, so the snapshot is entirely display-mode-
    /// independent — the caller can iterate `(width, height,
    /// dpi_scale)` combos without changing the real display mode.
    ///
    /// The PIDL / display-name caches are read from `self`, so the
    /// caller must have run `list_icons` first for icon bitmaps and
    /// labels to appear. Missing entries render as placeholder tiles
    /// (existing fallback behaviour).
    fn build_snapshot_plans(
        &mut self,
        dpi_scale: f32,
        positions: &[(IconId, Point)],
        render_options: OverlayRenderOptions,
    ) -> Vec<IconRenderPlan> {
        self.build_snapshot_plans_with_size(dpi_scale, positions, render_options, None)
    }

    fn build_snapshot_plans_with_size(
        &mut self, dpi_scale: f32, positions: &[(IconId, Point)],
        render_options: OverlayRenderOptions, icon_size: Option<u32>,
    ) -> Vec<IconRenderPlan> {
        // Identical geometry to the live path by construction — same
        // `IconLayout::resolve` call, same inputs, differing only in
        // where `icon_scale` comes from (caller-supplied here, resolved
        // per-monitor in `enrich_plans`). That equality is the entire
        // reason a snapshot can be used as an oracle for what the live
        // overlay draws. Do not inline this math back into either
        // caller.
        let base_size_dip = icon_size.map(|size| (size, size)).unwrap_or_else(|| self.query_icon_size_px().unwrap_or((48, 48)));
        let icon_scale = dpi_scale.max(0.01);
        let metrics = label_metrics_for_dpi(dpi_for_scale(icon_scale));
        let layout = IconLayout::resolve(base_size_dip, icon_scale, &metrics);

        positions
            .iter()
            .map(|(id, pt)| {
                let extracted = self.pidl_cache.get(id).and_then(|pidl| unsafe {
                    extract_icon_bitmap(
                        pidl.as_ptr(),
                        layout.size_px.0,
                        render_options.draw_shortcut_overlay,
                        render_options.draw_shield_overlay,
                    )
                });
                let (image, shortcut_arrow_image) = match extracted {
                    Some(bundle) => (Some(bundle.base), bundle.shortcut_arrow),
                    None => (None, None),
                };
                let label = if render_options.draw_labels {
                    self.display_name_cache
                        .get(id)
                        .cloned()
                        .map(|text| IconLabel {
                            text,
                            bounds_px: layout.label_bounds_px,
                        })
                } else {
                    None
                };
                IconRenderPlan {
                    effect: None,
                    stationary: false,
                    id: id.clone(),
                    source_position: *pt,
                    final_position: *pt,
                    size_px: layout.size_px,
                    image,
                    label,
                    render_offset_px: layout.render_offset_px,
                    shortcut_arrow_image,
                }
            })
            .collect()
    }

    /// Query the current desktop icon size in physical pixels via
    /// `IFolderView2::GetViewModeAndIconSize`.
    ///
    /// Returns `None` if the folder view is unreachable or the shell
    /// reports an implausible size — in that case callers should fall
    /// back to `(48, 48)`.
    fn query_icon_size_px(&mut self) -> Option<(u32, u32)> {
        let view = self.com.folder_view().ok()?;
        // SAFETY: valid `IFolderView2` interface pointer; out params
        // are stack-local.
        let mut mode = FOLDERVIEWMODE(0);
        let mut px: i32 = 0;
        let hr = unsafe { view.GetViewModeAndIconSize(&mut mode, &mut px) };
        if hr.is_err() || px <= 0 {
            return None;
        }
        // Clamp so a corrupt shell can't report an absurd size.
        let sz = (px as u32).clamp(16, 256);
        Some((sz, sz))
    }

    /// Locate the `SysListView32` child of the cached
    /// `SHELLDLL_DefView` window. Returns `None` if the shell view
    /// HWND isn't cached yet or the child isn't findable (which
    /// happens on non-standard shells).
    ///
    /// Used by [`commit_overlay_frame`](Self::commit_overlay_frame)
    /// as a `ShowWindow(SW_HIDE)` backup when `FWF_HIDEICONS` on the
    /// desktop folder view doesn't take effect (Windows 11 quirk).
    fn find_syslistview32(&self) -> Option<HWND> {
        let parent = self.com.shell_view_hwnd()?;
        // SAFETY: `FindWindowExW` takes a parent HWND and a class
        // name; it returns a child HWND or an error (child not
        // present). Both branches are safe.
        let class_name = w!("SysListView32");
        match unsafe { FindWindowExW(Some(parent), None, class_name, PCWSTR::null()) } {
            Ok(hwnd) if !hwnd.is_invalid() => Some(hwnd),
            _ => None,
        }
    }

    /// Teleport the real Shell icons to their final positions, ahead of
    /// hiding them. Returns the positions still awaiting a commit —
    /// empty on success, the untouched input on failure so
    /// `finalize_overlay_session` can retry.
    fn teleport_real_icons(
        &mut self,
        pending: Vec<(IconId, Point)>,
    ) -> Vec<(IconId, Point)> {
        if pending.is_empty() {
            return pending;
        }
        match self.set_positions(&pending) {
            Ok(_) => {
                debug!(icons = pending.len(), "teleported real icons to targets");
                Vec::new()
            }
            Err(e) => {
                warn!(
                    error = %e,
                    icons = pending.len(),
                    "first-frame set_positions failed; real icons will pop into \
                     place at animation end instead"
                );
                pending
            }
        }
    }

    /// Stop Explorer drawing the real icons, two ways, and return the
    /// guard that owns undoing both.
    ///
    /// `FWF_HIDEICONS` on the desktop folder view is documented as
    /// unreliable on Windows 11, so the `SysListView32` that actually
    /// paints the icons is hidden too. Either mechanism alone is
    /// enough; the guard restores whichever stuck.
    fn hide_real_icons(&mut self) -> ShellHideGuard {
        let mut guard = ShellHideGuard {
            hidden_syslistview: None,
            flag_set: false,
        };

        match self.apply_flags(FWF_HIDEICONS, FWF_HIDEICONS) {
            Ok(()) => guard.flag_set = true,
            Err(e) => warn!(
                error = %e,
                "failed to set FWF_HIDEICONS; falling back to SysListView32 hide only"
            ),
        }
        // Force Explorer to actually process the flag change.
        self.com.redraw_folder_view();

        if let Some(hwnd) = self.find_syslistview32() {
            // SAFETY: `ShowWindow` is safe on any HWND value.
            let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
            guard.hidden_syslistview = Some(hwnd);
        }
        debug!(
            flag_set = guard.flag_set,
            syslistview_hidden = guard.hidden_syslistview.is_some(),
            "real icons hidden"
        );
        guard
    }

    /// Undo everything [`Self::hide_real_icons`] did, consuming the guard.
    ///
    /// `SysListView32` is restored BEFORE `FWF_HIDEICONS` is cleared,
    /// so if the flag was the mechanism that actually hid the icons,
    /// the window restore happens under still-hidden semantics and
    /// cannot flash the old positions.
    ///
    /// `flag_set` is cleared **only** when the Shell call succeeds. A
    /// failed clear therefore stays visible to the guard's `Drop` and
    /// to the stale-flag sweep in `begin_overlay_session`, instead of
    /// being silently forgotten.
    fn restore_real_icons(&mut self, mut hide: ShellHideGuard) {
        hide.show_syslistview();

        if hide.flag_set {
            match self.apply_flags(FWF_HIDEICONS, 0) {
                Ok(()) => hide.flag_set = false,
                // Left `true` on purpose so the guard's Drop warns and
                // the next session's stale-flag sweep picks it up.
                Err(e) => tracing::error!(
                    error = %e,
                    "failed to clear FWF_HIDEICONS — desktop icons may stay hidden"
                ),
            }
        }
        debug!("real icons restored");
    }

    /// Poll `IFolderView2::GetItemPosition` until at least one of the
    /// moved icons reports a position within `CONFIRM_TOLERANCE_PX` of
    /// its target, or `SHELL_CONFIRM_TIMEOUT` expires. Returns the
    /// confirmed ids.
    ///
    /// The shell almost always updates its internal layout within a
    /// few milliseconds of `SelectAndPositionItems` returning, but
    /// that is not guaranteed.
    fn await_shell_confirmation(
        &mut self,
        final_positions: &[(IconId, Point)],
        missing: &[IconId],
    ) -> Vec<IconId> {
        let missing_set: std::collections::HashSet<&IconId> = missing.iter().collect();
        let view = match self.com.folder_view() {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };

        let deadline = Instant::now() + SHELL_CONFIRM_TIMEOUT;
        let mut confirmed: Vec<IconId> = Vec::new();
        const CONFIRM_TOLERANCE_PX: i32 = 4;

        loop {
            for (id, target) in final_positions {
                if missing_set.contains(id) {
                    continue;
                }
                if confirmed.iter().any(|c| c == id) {
                    continue;
                }
                let Some(pidl) = self.pidl_cache.get(id) else {
                    continue;
                };
                // SAFETY: `pidl` is owned by the PIDL cache and `view`
                // is a clone of the cached `IFolderView2`; both stay
                // valid for the duration of this call.
                let pos = unsafe { view.GetItemPosition(pidl.as_ptr()) };
                if let Ok(p) = pos {
                    let dx = (p.x - target.x).abs();
                    let dy = (p.y - target.y).abs();
                    if dx <= CONFIRM_TOLERANCE_PX && dy <= CONFIRM_TOLERANCE_PX {
                        confirmed.push(id.clone());
                        return confirmed;
                    }
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(SHELL_CONFIRM_POLL);
        }
        confirmed
    }
}

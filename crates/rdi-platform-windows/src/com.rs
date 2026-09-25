//! COM initialisation and desktop `IFolderView2` acquisition.
//!
//! Windows' Explorer exposes the desktop as a folder view. Reading or
//! moving icon positions needs an `IFolderView2` pointer at that view.
//! Getting it is a five-step COM dance:
//!
//! ```text
//! CoCreateInstance(ShellWindows)
//!   → IShellWindows::FindWindowSW(CSIDL_DESKTOP, SWC_DESKTOP, SWFO_NEEDDISPATCH)
//!   → cast IDispatch to IServiceProvider
//!   → IServiceProvider::QueryService(SID_STopLevelBrowser)
//!   → IShellBrowser::QueryActiveShellView
//!   → cast IShellView to IFolderView2
//! ```
//!
//! The resulting interface is cached. When a call through it fails the
//! cache **must** be invalidated, because Explorer replaces its view
//! object across certain state changes (e.g. after `FWF_NOICONS` is
//! toggled, or when the shell restarts).
//!
//! The [`ComContext`] owns both the initialisation state and the cached
//! view, and lives on the engine's worker thread — no cross-thread
//! sharing.

use rdi_core::DesktopError;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    RDW_ALLCHILDREN, RDW_INVALIDATE, RDW_UPDATENOW, RedrawWindow,
};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IServiceProvider,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{
    IFolderView2, IShellBrowser, IShellView, IShellWindows, SID_STopLevelBrowser, SWC_DESKTOP,
    SWFO_NEEDDISPATCH, ShellWindows,
};
use windows::core::Interface;

/// `CSIDL_DESKTOP` — passed as a `VARIANT` to `IShellWindows::FindWindowSW`.
const CSIDL_DESKTOP: i32 = 0x0000;

/// Owns the per-thread COM initialisation and a cached `IFolderView2`.
pub(crate) struct ComContext {
    /// `true` once `CoInitializeEx` has succeeded on the current
    /// thread. `false` if initialisation failed, in which case `Drop`
    /// skips the uninitialise.
    co_initialized: bool,
    /// The cached desktop folder view. `None` means either "never
    /// acquired" or "invalidated after a failure".
    folder_view: Option<IFolderView2>,
    /// HWND of the shell view that hosts the desktop icons — the child
    /// `SHELLDLL_DefView` window inside `Progman` (or a `WorkerW`).
    /// Captured at the same time as [`Self::folder_view`] and used to
    /// force Explorer to repaint after every position commit — see
    /// [`Self::redraw_folder_view`].
    folder_hwnd: Option<HWND>,
}

impl ComContext {
    /// Build a fresh (uninitialised) context. Call [`Self::folder_view`]
    /// to lazily initialise COM and acquire the view.
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            co_initialized: false,
            folder_view: None,
            folder_hwnd: None,
        }
    }

    /// Ensure the current thread has COM initialised as an STA.
    ///
    /// `S_FALSE` (thread already initialised) is treated as success —
    /// matches the C++ / legacy behaviour and the documented contract
    /// of `CoInitializeEx`.
    fn ensure_co_initialized(&mut self) -> Result<(), DesktopError> {
        if self.co_initialized {
            return Ok(());
        }
        // SAFETY: `CoInitializeEx` is one of the safest COM calls; the
        // only precondition is that no interface pointers survive from
        // a prior mismatched initialisation, which the
        // `co_initialized` flag guarantees.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr.is_err() {
            return Err(DesktopError::BackendUnavailable(format!(
                "CoInitializeEx failed: {hr:?}"
            )));
        }
        self.co_initialized = true;
        Ok(())
    }

    /// Return the cached `IFolderView2`, acquiring it on first use.
    pub(crate) fn folder_view(&mut self) -> Result<IFolderView2, DesktopError> {
        self.ensure_co_initialized()?;
        if let Some(view) = &self.folder_view {
            return Ok(view.clone());
        }
        let (view, hwnd) = find_desktop_folder_view()?;
        self.folder_view = Some(view.clone());
        self.folder_hwnd = Some(hwnd);
        Ok(view)
    }

    /// Ask the shell to repaint the desktop's folder view **right now**.
    ///
    /// `IFolderView2::SelectAndPositionItems` runs dozens of times per
    /// second between animation ticks. Explorer accepts the new
    /// positions but defers the repaint — its `SysListView32` batches
    /// invalidations and only pushes them to the screen once its
    /// message loop goes idle, which never happens during a rapid
    /// commit stream. The visible result is frozen icons that jump to
    /// the final position when the animation ends.
    ///
    /// `RedrawWindow(RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW)`
    /// forces the invalidation to be posted AND processed
    /// synchronously. `RDW_ALLCHILDREN` matters because the visible
    /// icons live in a child window of the folder view.
    ///
    /// Best-effort: any failure is silently ignored so a transient GDI
    /// hiccup can't kill an in-flight animation.
    #[inline]
    pub(crate) fn redraw_folder_view(&self) {
        let Some(hwnd) = self.folder_hwnd else {
            tracing::debug!("RedrawWindow skipped: no cached folder-view HWND");
            return;
        };
        // SAFETY: `hwnd` was obtained from `IShellView::GetWindow` on
        // the folder view still held here. Passing `None` for the
        // update rect / region redraws the entire client area.
        let ok = unsafe {
            RedrawWindow(
                Some(hwnd),
                None,
                None,
                RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW,
            )
        };
        if !ok.as_bool() {
            // `RedrawWindow` returns FALSE on failure and sets the
            // thread's last-error code. `Error::from_thread()` snapshots
            // it as an HRESULT and `message()` formats it via
            // `FormatMessageW`.
            let err = windows::core::Error::from_thread();
            tracing::warn!(
                hwnd = format_args!("0x{:x}", hwnd.0 as usize),
                hresult = format_args!("0x{:08x}", err.code().0 as u32),
                message = %err.message(),
                "RedrawWindow returned FALSE"
            );
        }
    }

    /// Drop the cached view. Called after any failing COM call so the
    /// next request re-acquires a fresh interface. Matches
    /// `legacy_lib::invalidate_folder_view`.
    #[inline]
    pub(crate) fn invalidate(&mut self) {
        self.folder_view = None;
        self.folder_hwnd = None;
    }

    /// Return the cached `SHELLDLL_DefView` HWND, if one was acquired.
    ///
    /// Used by [`WindowsBackend`](crate::WindowsBackend) to locate the
    /// child `SysListView32` window it hides during an overlay
    /// animation.
    #[inline]
    pub(crate) fn shell_view_hwnd(&self) -> Option<HWND> {
        self.folder_hwnd
    }

    /// Return `false` when a shell-view HWND is cached but no longer
    /// identifies a valid window. `None` yields `true` — nothing to
    /// check yet, so nothing to cancel.
    ///
    /// Used by [`WindowsBackend::commit_overlay_frame`] as a
    /// watchdog for the Explorer-restart / hostile-shell-swap case
    /// where `TaskbarCreated` never fires but the shell view HWND
    /// silently becomes invalid.
    #[inline]
    pub(crate) fn is_shell_view_valid(&self) -> bool {
        match self.folder_hwnd {
            None => true,
            // SAFETY: `IsWindow` is safe on any HWND value — it
            // returns FALSE for invalid handles rather than
            // dereferencing them.
            Some(hwnd) => unsafe {
                windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(hwnd))
            }
            .as_bool(),
        }
    }
}

impl Drop for ComContext {
    fn drop(&mut self) {
        // Release the cached view first so its `Release()` runs while
        // the apartment is still valid.
        self.folder_view = None;
        self.folder_hwnd = None;
        if self.co_initialized {
            // SAFETY: paired with the earlier `CoInitializeEx` call on
            // this thread.
            unsafe { CoUninitialize() };
            self.co_initialized = false;
        }
    }
}

/// Walk the shell to find the desktop's `IFolderView2` **and** the
/// HWND of the folder view window it renders into, mirroring the
/// legacy `find_desktop_folder_view` helper.
fn find_desktop_folder_view() -> Result<(IFolderView2, HWND), DesktopError> {
    // SAFETY: every call below sits in a single `unsafe` block so the
    // whole COM dance is annotated once — each individual method is
    // `unsafe` because it dereferences a COM v-table pointer, and that
    // pointer is valid because it came from the previous successful
    // call in the chain.
    unsafe {
        let shell_windows: IShellWindows =
            CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER).map_err(|e| {
                DesktopError::BackendUnavailable(format!(
                    "CoCreateInstance(ShellWindows) failed: {e}"
                ))
            })?;

        let loc = VARIANT::from(CSIDL_DESKTOP);
        let empty = VARIANT::default();
        let mut lhwnd = 0i32;

        let dispatch = shell_windows
            .FindWindowSW(&loc, &empty, SWC_DESKTOP, &mut lhwnd, SWFO_NEEDDISPATCH)
            .map_err(|e| {
                DesktopError::BackendUnavailable(format!(
                    "IShellWindows::FindWindowSW(CSIDL_DESKTOP) failed: {e}"
                ))
            })?;

        let service_provider: IServiceProvider = dispatch.cast().map_err(|e| {
            DesktopError::BackendUnavailable(format!(
                "IDispatch → IServiceProvider cast failed: {e}"
            ))
        })?;

        let browser: IShellBrowser = service_provider
            .QueryService(&SID_STopLevelBrowser)
            .map_err(|e| {
                DesktopError::BackendUnavailable(format!(
                    "IServiceProvider::QueryService(SID_STopLevelBrowser) failed: {e}"
                ))
            })?;

        let view: IShellView = browser.QueryActiveShellView().map_err(|e| {
            DesktopError::BackendUnavailable(format!(
                "IShellBrowser::QueryActiveShellView failed: {e}"
            ))
        })?;

        // Grab the shell view's HWND before `view` is consumed by the
        // cast below. `IShellView` derives from `IOleWindow`, and the
        // Rust bindings expose `GetWindow` on `IShellView` directly via
        // `Deref` (see `windows_core::imp::interface_hierarchy!` on
        // `IShellView` in the `windows` crate). Some Explorer
        // shell-view implementations refuse a QI for `IOleWindow` even
        // though they *derive* from it, so deref — never cast.
        let hwnd = view.GetWindow().map_err(|e| {
            DesktopError::BackendUnavailable(format!(
                "IShellView::GetWindow failed: {e}"
            ))
        })?;

        let folder_view: IFolderView2 = view.cast().map_err(|e| {
            DesktopError::BackendUnavailable(format!(
                "IShellView → IFolderView2 cast failed: {e}"
            ))
        })?;

        Ok((folder_view, hwnd))
    }
}

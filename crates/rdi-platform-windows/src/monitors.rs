//! Multi-monitor enumeration via `EnumDisplayMonitors` + `GetMonitorInfoW`.
//!
//! Populates [`rdi_core::MonitorInfo`] entries in the same virtual-
//! screen coordinate system that [`crate::WindowsBackend::list_icons`]
//! uses, so callers can reason about "which monitor is this icon on?"
//! without a second coordinate transform.
//!
//! DPI is queried per-monitor via [`GetDpiForMonitor`] when available;
//! failures degrade to `scale_factor = 1.0`.

use std::mem::size_of;

use rdi_core::{DesktopError, MonitorInfo, Rect as CoreRect};
use windows::Win32::Foundation::{LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::core::BOOL;

use crate::pidl::wcslen_bounded;

/// `MONITORINFOF_PRIMARY` — SDK-documented value `0x00000001`, not
/// exposed by the `windows` crate.
const MONITORINFOF_PRIMARY: u32 = 0x0000_0001;

/// Query every attached display and return one [`MonitorInfo`] per
/// monitor in the virtual-screen coordinate space.
pub(crate) fn enumerate_monitors() -> Result<Vec<MonitorInfo>, DesktopError> {
    let mut monitors: Vec<MonitorInfo> = Vec::new();

    // The callback pushes into `monitors` through the LPARAM slot. The
    // vector lives for the entire `EnumDisplayMonitors` call, so the
    // pointer is valid throughout.
    let param = LPARAM(&mut monitors as *mut Vec<MonitorInfo> as isize);

    // SAFETY: `EnumDisplayMonitors(None, None, cb, param)` synchronously
    // invokes `cb` once per monitor on the calling thread and then
    // returns. `cb` receives the `LPARAM` verbatim and treats it as a
    // pointer to `Vec<MonitorInfo>` (see [`monitor_enum_proc`]).
    let ok: BOOL = unsafe { EnumDisplayMonitors(None, None, Some(monitor_enum_proc), param) };
    if ok != TRUE {
        return Err(DesktopError::BackendUnavailable(
            "EnumDisplayMonitors returned FALSE".into(),
        ));
    }
    Ok(monitors)
}

/// `MONITORENUMPROC` — invoked once per display by `EnumDisplayMonitors`.
///
/// # Safety
/// `data` must be the `LPARAM` passed to `EnumDisplayMonitors` — a
/// non-null pointer to a live `Vec<MonitorInfo>`. `hmon` is a valid
/// `HMONITOR` for the duration of the call.
unsafe extern "system" fn monitor_enum_proc(
    hmon: HMONITOR,
    _hdc: HDC,
    _lprect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // SAFETY: `data` was constructed in `enumerate_monitors` above and
    // points at a live `Vec<MonitorInfo>` on the caller's stack.
    let out: &mut Vec<MonitorInfo> = unsafe { &mut *(data.0 as *mut Vec<MonitorInfo>) };

    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;

    // SAFETY: `GetMonitorInfoW` writes into `info` iff it returns TRUE.
    // Failure skips this monitor rather than aborting enumeration.
    let ok = unsafe {
        GetMonitorInfoW(hmon, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO)
    };
    if ok != TRUE {
        return TRUE;
    }

    let bounds = rect_from_win32(info.monitorInfo.rcMonitor);
    let work_area = rect_from_win32(info.monitorInfo.rcWork);
    let is_primary = (info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0;

    let name_len = wcslen_bounded(&info.szDevice, info.szDevice.len());
    let name = String::from_utf16_lossy(&info.szDevice[..name_len]);

    let scale_factor = query_scale_factor(hmon);

    out.push(MonitorInfo {
        id: name.clone(),
        name,
        bounds,
        work_area,
        is_primary,
        scale_factor,
    });

    TRUE
}

fn rect_from_win32(r: RECT) -> CoreRect {
    CoreRect::new(r.left, r.top, r.right, r.bottom)
}

/// Query the effective per-monitor DPI. Silent 1.0 fallback if the
/// system refuses (e.g. running on an OS without per-monitor DPI).
fn query_scale_factor(hmon: HMONITOR) -> f32 {
    let mut dpi_x: u32 = 0;
    let mut dpi_y: u32 = 0;
    // SAFETY: `GetDpiForMonitor` reads only from `hmon` (valid inside
    // the enumeration callback) and writes only into the two `u32`s.
    let hr = unsafe {
        GetDpiForMonitor(
            hmon,
            MDT_EFFECTIVE_DPI,
            &mut dpi_x as *mut u32,
            &mut dpi_y as *mut u32,
        )
    };
    if hr.is_err() || dpi_x == 0 {
        return 1.0;
    }
    dpi_x as f32 / 96.0
}

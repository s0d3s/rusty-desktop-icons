//! Monitor / display enumeration types.
//!
//! Icon coordinates on Windows live in **virtual-screen space**: a
//! single coordinate system that spans every attached monitor. The
//! primary monitor's top-left is `(0, 0)`; secondary monitors can sit
//! anywhere around it, including at negative offsets when arranged to
//! the left of or above the primary.
//!
//! To place icons intelligently on specific monitors, callers need to
//! know where each monitor lives in that shared coordinate space —
//! that is what [`MonitorInfo`] carries.
//!
//! Populated by [`DesktopBackend::list_monitors`](crate::DesktopBackend::list_monitors).

use crate::Point;

/// A pixel-space axis-aligned rectangle.
///
/// Uses inclusive-left / exclusive-right / inclusive-top / exclusive-
/// bottom conventions, matching Windows `RECT`. A monitor whose upper-
/// left is `(0, 0)` and whose size is `1920 x 1080` therefore has
/// `right = 1920` and `bottom = 1080`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    /// Construct a `Rect` from its four edges.
    #[inline]
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// Construct a `Rect` from its top-left corner and its size.
    #[inline]
    pub const fn from_origin_size(left: i32, top: i32, width: i32, height: i32) -> Self {
        Self {
            left,
            top,
            right: left + width,
            bottom: top + height,
        }
    }

    #[inline]
    pub const fn width(&self) -> i32 {
        self.right - self.left
    }

    #[inline]
    pub const fn height(&self) -> i32 {
        self.bottom - self.top
    }

    /// True if `point` lies inside the rectangle (inclusive left/top,
    /// exclusive right/bottom — matches Windows `PtInRect`).
    #[inline]
    pub const fn contains(&self, point: Point) -> bool {
        point.x >= self.left
            && point.x < self.right
            && point.y >= self.top
            && point.y < self.bottom
    }
}

/// A single connected display in the virtual-screen coordinate system.
#[derive(Clone, Debug, PartialEq)]
pub struct MonitorInfo {
    /// Stable-for-session identifier. On Windows this is the device
    /// name reported by `GetMonitorInfoW` — for example `\\.\DISPLAY1`.
    /// Stable across process runs on the same session; **not**
    /// guaranteed to be stable across reboots or hardware changes.
    pub id: String,

    /// Human-readable display name (same as `id` on the current Windows
    /// backend; kept as a distinct field so future backends can supply
    /// a friendlier label).
    pub name: String,

    /// Full monitor bounds in virtual-screen coordinates. May contain
    /// negative left/top when the monitor is arranged to the left of
    /// or above the primary monitor.
    pub bounds: Rect,

    /// Bounds excluding the taskbar and any other reserved-space
    /// widgets. This is what icons should honour when the user has a
    /// visible taskbar.
    pub work_area: Rect,

    /// True for the monitor that owns virtual coordinate `(0, 0)`.
    pub is_primary: bool,

    /// Effective DPI scale factor for this monitor, expressed as a
    /// multiplier (`1.0` = 96 DPI, `1.5` = 144 DPI, `2.0` = 192 DPI,
    /// …).
    ///
    /// This value is reported in the **calling process's coordinate
    /// space**. A DPI-unaware process on a HiDPI monitor sees
    /// scale = 1.0 (Windows virtualises the coordinates so 96 DPI is
    /// always accurate for what the process draws); a per-monitor-DPI-
    /// aware process sees the true scale. Since icon coordinates use
    /// the same space, callers can multiply/divide safely without a
    /// second correction.
    ///
    /// `1.0` on backends that do not implement per-monitor DPI.
    pub scale_factor: f32,
}

impl MonitorInfo {
    pub fn resolution(&self) -> Point {
        Point::new(self.bounds.width(), self.bounds.height())
    }

    pub fn dpi(&self) -> f32 {
        self.scale_factor * 96.0
    }

    /// True if the given point (in virtual-screen coordinates) lies
    /// inside this monitor's full bounds.
    #[inline]
    pub fn contains(&self, point: Point) -> bool {
        self.bounds.contains(point)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_from_origin_size_matches_edges() {
        let r = Rect::from_origin_size(-100, -50, 1920, 1080);
        assert_eq!(r.left, -100);
        assert_eq!(r.top, -50);
        assert_eq!(r.right, 1820);
        assert_eq!(r.bottom, 1030);
        assert_eq!(r.width(), 1920);
        assert_eq!(r.height(), 1080);
    }

    #[test]
    fn rect_contains_half_open() {
        let r = Rect::new(0, 0, 100, 100);
        assert!(r.contains(Point::new(0, 0))); // top-left is inclusive
        assert!(r.contains(Point::new(99, 99)));
        assert!(!r.contains(Point::new(100, 50))); // right is exclusive
        assert!(!r.contains(Point::new(50, 100))); // bottom is exclusive
        assert!(!r.contains(Point::new(-1, 50)));
    }

    #[test]
    fn monitor_contains_delegates_to_bounds() {
        let m = MonitorInfo {
            id: "\\.\\DISPLAY2".into(),
            name: "\\.\\DISPLAY2".into(),
            bounds: Rect::from_origin_size(-1920, 0, 1920, 1080),
            work_area: Rect::from_origin_size(-1920, 0, 1920, 1040),
            is_primary: false,
            scale_factor: 1.0,
        };
        assert!(m.contains(Point::new(-100, 100)));
        assert!(!m.contains(Point::new(100, 100)));
    }
}

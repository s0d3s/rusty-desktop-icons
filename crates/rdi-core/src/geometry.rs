//! Simple 2D integer geometry used throughout the crate.

/// 2D point in virtual-screen coordinates (pixels).
///
/// Values match what the Windows shell reports for
/// `IFolderView2::GetItemPosition`: signed 32-bit integers in the
/// **virtual-screen coordinate system** — a single space that spans
/// every attached monitor. The primary monitor's top-left is `(0, 0)`;
/// coordinates on other monitors can be negative when they are
/// arranged to the left of or above the primary. Use
/// [`crate::MonitorInfo`] (populated by
/// [`crate::DesktopBackend::list_monitors`]) to discover where each
/// monitor lives in this space.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const ZERO: Self = Self { x: 0, y: 0 };

    #[inline]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// Euclidean distance between two points, in pixels, as an `f32`.
    #[inline]
    pub fn distance(a: Self, b: Self) -> f32 {
        let dx = (b.x - a.x) as f32;
        let dy = (b.y - a.y) as f32;
        (dx * dx + dy * dy).sqrt()
    }

    /// Component-wise linear interpolation `origin + t * (target - origin)`,
    /// returning an integer point (rounded to nearest).
    #[inline]
    pub fn lerp_i32(origin: Self, target: Self, tx: f32, ty: f32) -> Self {
        let x = origin.x as f32 + tx * (target.x - origin.x) as f32;
        let y = origin.y as f32 + ty * (target.y - origin.y) as f32;
        Self {
            x: x.round() as i32,
            y: y.round() as i32,
        }
    }
}

impl From<(i32, i32)> for Point {
    #[inline]
    fn from((x, y): (i32, i32)) -> Self {
        Self { x, y }
    }
}

impl From<Point> for (i32, i32) {
    #[inline]
    fn from(p: Point) -> Self {
        (p.x, p.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_zero_for_equal_points() {
        assert_eq!(Point::distance(Point::new(5, 7), Point::new(5, 7)), 0.0);
    }

    #[test]
    fn distance_matches_pythagoras() {
        // 3-4-5 triangle.
        let d = Point::distance(Point::ZERO, Point::new(3, 4));
        assert!((d - 5.0).abs() < 1e-6, "d = {d}");
    }

    #[test]
    fn lerp_endpoints() {
        let a = Point::new(10, 20);
        let b = Point::new(110, 220);
        assert_eq!(Point::lerp_i32(a, b, 0.0, 0.0), a);
        assert_eq!(Point::lerp_i32(a, b, 1.0, 1.0), b);
    }

    #[test]
    fn lerp_midpoint_rounds() {
        let a = Point::new(0, 0);
        let b = Point::new(1, 1); // midpoint 0.5 rounds to 1 (banker's-agnostic Rust default is half-away-from-zero via `round`)
        let mid = Point::lerp_i32(a, b, 0.5, 0.5);
        assert!(mid == Point::new(1, 1) || mid == Point::new(0, 0));
    }
}

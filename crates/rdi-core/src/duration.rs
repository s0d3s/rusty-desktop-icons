//! Duration policies for a single icon's animation.

use std::time::Duration as StdDuration;

use crate::{DesktopError, Point};

/// Minimum duration returned by [`Duration::resolve`] even when the
/// distance would compute to zero, so the tick loop never has to divide
/// by zero.
const MIN_RESOLVED: StdDuration = StdDuration::from_millis(1);

/// How the animation engine determines how long a single icon takes to
/// move from its origin to its target.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Duration {
    /// A fixed wall-clock duration.
    Fixed(StdDuration),

    /// Duration derived from movement distance and a constant speed,
    /// optionally clamped to `min` / `max`.
    ///
    /// * `speed_px_per_sec` must be strictly positive and finite.
    /// * `min` and `max`, if both provided, must satisfy `min <= max`.
    Distance {
        speed_px_per_sec: f32,
        min: Option<StdDuration>,
        max: Option<StdDuration>,
    },
}

impl Duration {
    /// Convenience constructor for [`Duration::Fixed`].
    #[inline]
    pub const fn fixed(d: StdDuration) -> Self {
        Self::Fixed(d)
    }

    /// Convenience constructor for [`Duration::Distance`] without clamps.
    #[inline]
    pub const fn distance(speed_px_per_sec: f32) -> Self {
        Self::Distance {
            speed_px_per_sec,
            min: None,
            max: None,
        }
    }

    /// Convenience constructor for a clamped distance-based duration.
    #[inline]
    pub const fn distance_clamped(
        speed_px_per_sec: f32,
        min: StdDuration,
        max: StdDuration,
    ) -> Self {
        Self::Distance {
            speed_px_per_sec,
            min: Some(min),
            max: Some(max),
        }
    }

    /// Resolve this policy to a concrete duration for the given movement.
    ///
    /// Returns [`DesktopError::InvalidDuration`] if the policy contains
    /// invalid values (non-positive fixed duration, non-positive speed,
    /// `min > max`, non-finite parameters).
    pub fn resolve(&self, from: Point, to: Point) -> Result<StdDuration, DesktopError> {
        match *self {
            Duration::Fixed(d) => {
                if d.is_zero() {
                    Err(DesktopError::InvalidDuration(
                        "fixed duration must be non-zero".into(),
                    ))
                } else {
                    Ok(d)
                }
            }
            Duration::Distance {
                speed_px_per_sec,
                min,
                max,
            } => {
                if !speed_px_per_sec.is_finite() || speed_px_per_sec <= 0.0 {
                    return Err(DesktopError::InvalidDuration(format!(
                        "distance duration requires positive finite speed, got {speed_px_per_sec}"
                    )));
                }
                if let (Some(lo), Some(hi)) = (min, max) {
                    if lo > hi {
                        return Err(DesktopError::InvalidDuration(format!(
                            "distance duration min ({lo:?}) is greater than max ({hi:?})"
                        )));
                    }
                }

                let dist = Point::distance(from, to);
                let seconds = (dist / speed_px_per_sec).max(0.0);
                let mut d = StdDuration::try_from_secs_f32(seconds).unwrap_or(StdDuration::ZERO);

                if let Some(lo) = min {
                    if d < lo {
                        d = lo;
                    }
                }
                if let Some(hi) = max {
                    if d > hi {
                        d = hi;
                    }
                }

                // Never return 0 — even a zero-distance move needs to
                // produce a well-defined tick loop.
                if d.is_zero() {
                    d = MIN_RESOLVED;
                }
                Ok(d)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_returns_value_verbatim() {
        let d = Duration::fixed(StdDuration::from_millis(500));
        assert_eq!(
            d.resolve(Point::ZERO, Point::new(999, 0)).unwrap(),
            StdDuration::from_millis(500)
        );
    }

    #[test]
    fn fixed_zero_is_rejected() {
        let d = Duration::fixed(StdDuration::ZERO);
        let err = d.resolve(Point::ZERO, Point::ZERO).unwrap_err();
        assert!(matches!(err, DesktopError::InvalidDuration(_)));
    }

    #[test]
    fn distance_scales_with_distance() {
        // 500 px @ 1000 px/s = 0.5 s.
        let d = Duration::distance(1000.0);
        let out = d.resolve(Point::ZERO, Point::new(300, 400)).unwrap();
        // Allow a couple of ms slack for f32 rounding.
        let expected = StdDuration::from_millis(500);
        assert!(
            (out.as_secs_f64() - expected.as_secs_f64()).abs() < 0.01,
            "got {out:?}"
        );
    }

    #[test]
    fn distance_zero_falls_back_to_min_resolved() {
        let d = Duration::distance(1000.0);
        let out = d.resolve(Point::ZERO, Point::ZERO).unwrap();
        assert_eq!(out, MIN_RESOLVED);
    }

    #[test]
    fn distance_min_clamps_short_moves() {
        let d = Duration::distance_clamped(
            1000.0,
            StdDuration::from_millis(300),
            StdDuration::from_secs(2),
        );
        // 10 px / 1000 px/s = 10 ms → clamped up to 300 ms.
        let out = d.resolve(Point::ZERO, Point::new(10, 0)).unwrap();
        assert_eq!(out, StdDuration::from_millis(300));
    }

    #[test]
    fn distance_max_clamps_long_moves() {
        let d = Duration::distance_clamped(
            1.0, // 1 px/s → very slow
            StdDuration::from_millis(100),
            StdDuration::from_millis(500),
        );
        let out = d.resolve(Point::ZERO, Point::new(5000, 0)).unwrap();
        assert_eq!(out, StdDuration::from_millis(500));
    }

    #[test]
    fn distance_bad_speed_rejected() {
        for &s in &[0.0f32, -1.0, f32::NAN, f32::INFINITY] {
            let d = Duration::distance(s);
            let err = d.resolve(Point::ZERO, Point::new(10, 0)).unwrap_err();
            assert!(matches!(err, DesktopError::InvalidDuration(_)), "s = {s}");
        }
    }

    #[test]
    fn distance_min_greater_than_max_rejected() {
        let d = Duration::Distance {
            speed_px_per_sec: 100.0,
            min: Some(StdDuration::from_secs(2)),
            max: Some(StdDuration::from_secs(1)),
        };
        assert!(matches!(
            d.resolve(Point::ZERO, Point::new(10, 0)).unwrap_err(),
            DesktopError::InvalidDuration(_)
        ));
    }
}

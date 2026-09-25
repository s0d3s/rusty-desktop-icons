//! Ready-made procedural curves.
//!
//! These are thin wrappers over [`crate::curve::KeyframeCurve::from_curve_fn`]
//! that ship a handful of well-tuned non-easing shapes (spring, bounce,
//! elastic, overshoot / "back"). Users who need something more custom can
//! call [`crate::Curve::from_curve_fn`] / [`crate::Curve::from_motion_fn`]
//! directly.
//!
//! All functions return a fully validated [`crate::Curve`]; construction may
//! fail with [`crate::CurveError`] if the parameters yield an unsafe curve
//! (NaN, exploding values, ...).

use core::f32::consts::PI;

use crate::curve::{Curve, KeyframeInterp};
use crate::CurveError;

/// Damped-cosine "spring" curve.
///
/// * `damping` — fraction of critical damping in `[0, 1]`. `0.0` = pure
///   oscillation, `1.0` = critically damped.
/// * `stiffness`, `mass` — control the natural angular frequency
///   `ω = sqrt(stiffness / mass)`.
///
/// The curve starts at `0`, overshoots `1` (unless critically damped) and
/// settles near `1` at `t = 1`. It is normalised so the shape doesn't
/// depend on the animation's wall-clock duration.
pub fn spring(damping: f32, stiffness: f32, mass: f32) -> Result<Curve, CurveError> {
    let mass = mass.max(1e-3);
    let stiffness = stiffness.max(1e-3);
    let zeta = damping.clamp(0.0, 1.0);
    let omega = (stiffness / mass).sqrt();
    let omega_d = omega * (1.0 - zeta * zeta).max(0.0).sqrt();

    // Scale the closure's time so ~3 oscillations at zeta = 0 are visible
    // over t in [0, 1]. This is a heuristic default: the user still gets
    // full physics control via damping/stiffness/mass.
    let scale = 3.0 * PI;

    Curve::from_curve_fn(
        move |t| {
            let x = t * scale;
            1.0 - (-zeta * omega * x).exp() * (omega_d * x).cos()
        },
        128,
        KeyframeInterp::Linear,
    )
}

/// Bouncing curve — several damped rebounds converging on `1`.
///
/// * `bounces` — number of rebounds (≥ 1).
/// * `decay` — per-bounce amplitude loss in `(0, 1]` (higher = faster
///   settle). Values ≤ 0 are coerced to a small positive number so the
///   curve still lands exactly on `1` at `t = 1`.
pub fn bounce(bounces: u32, decay: f32) -> Result<Curve, CurveError> {
    let b = bounces.max(1) as f32;
    let decay = decay.clamp(0.0, 1.0).max(1e-2);
    // Envelope exponent — always positive so `(1-t)^k` hits 0 at t = 1
    // and the curve reaches exactly 1 there regardless of `bounces`.
    let k = 0.5 + 2.0 * decay;

    Curve::from_curve_fn(
        move |t| {
            let envelope = (1.0 - t).max(0.0).powf(k);
            1.0 - (b * PI * t).cos().abs() * envelope
        },
        128,
        KeyframeInterp::Linear,
    )
}

/// Elastic ease-out — sinusoidal overshoot damped by `2^(-10t)`.
///
/// * `period` — normalized oscillation period (default around `0.3`).
/// * `amplitude` — overshoot amplitude (≥ 1).
pub fn elastic(period: f32, amplitude: f32) -> Result<Curve, CurveError> {
    let p = period.max(0.05);
    let a = amplitude.max(1.0);
    // s shifts the sine so the curve starts at 0. Standard Robert Penner form.
    let s = p / (2.0 * PI) * (1.0f32 / a).asin();

    Curve::from_curve_fn(
        move |t| {
            if t <= 0.0 {
                return 0.0;
            }
            if t >= 1.0 {
                return 1.0;
            }
            a * (2.0f32).powf(-10.0 * t) * ((t - s) * 2.0 * PI / p).sin() + 1.0
        },
        128,
        KeyframeInterp::Linear,
    )
}

/// Back / overshoot ease-in-out (Robert Penner's "back" family).
///
/// * `strength` ≥ 0. `0.0` = no overshoot, `1.7` ≈ default CSS `back`.
pub fn overshoot(strength: f32) -> Result<Curve, CurveError> {
    let s = strength.max(0.0);
    let c1 = s + 1.0;

    Curve::from_curve_fn(
        move |t| {
            let u = t - 1.0;
            u * u * (c1 * u + s) + 1.0
        },
        96,
        KeyframeInterp::Linear,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AnimationCurve;

    const EPS: f32 = 1e-3;

    fn endpoints_reach_zero_and_one(curve: &Curve) {
        assert!(curve.eval(0.0).abs() < EPS, "start != 0: {}", curve.eval(0.0));
        assert!(
            (curve.eval(1.0) - 1.0).abs() < EPS,
            "end != 1: {}",
            curve.eval(1.0)
        );
    }

    #[test]
    fn spring_endpoints_and_bounded() {
        let c = spring(0.5, 180.0, 1.0).unwrap();
        endpoints_reach_zero_and_one(&c);
        // Values must stay within the runtime clamp so linear-value mapping is sane.
        for i in 0..=50 {
            let t = i as f32 / 50.0;
            let v = c.eval(t);
            assert!(v.abs() < 5.0, "spring blew up at t={t}: {v}");
        }
    }

    #[test]
    fn spring_critical_damping_no_overshoot() {
        let c = spring(1.0, 180.0, 1.0).unwrap();
        for i in 0..=50 {
            let t = i as f32 / 50.0;
            let v = c.eval(t);
            assert!(v <= 1.01, "critical spring overshot at t={t}: {v}");
        }
    }

    #[test]
    fn bounce_endpoints() {
        let c = bounce(3, 0.5).unwrap();
        endpoints_reach_zero_and_one(&c);
    }

    #[test]
    fn elastic_endpoints() {
        let c = elastic(0.3, 1.0).unwrap();
        endpoints_reach_zero_and_one(&c);
    }

    #[test]
    fn overshoot_zero_strength_is_smooth_ease_out_cubic() {
        let c = overshoot(0.0).unwrap();
        // With s=0 the polynomial u²·(1·u) + 1 = (t−1)³ + 1 = cubic-out.
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            let expected = {
                let u = 1.0 - t;
                1.0 - u * u * u
            };
            assert!(
                (c.eval(t) - expected).abs() < 5e-3,
                "at t={t}: got {}, want {expected}",
                c.eval(t)
            );
        }
    }

    #[test]
    fn overshoot_positive_strength_overshoots_one() {
        let c = overshoot(1.7).unwrap();
        // Somewhere in the middle it should exceed 1.
        let peak = (0..=100)
            .map(|i| c.eval(i as f32 / 100.0))
            .fold(f32::MIN, f32::max);
        assert!(peak > 1.05, "no overshoot detected, peak = {peak}");
    }
}

//! Animation curves.
//!
//! Every axis of every icon animation is driven by a value in `[0, 1]`
//! produced by an [`AnimationCurve`]. The engine linearly maps this value
//! from the origin to the target coordinate, so overshoot curves (values
//! outside `[0, 1]`) are permitted.
//!
//! Two concrete implementations share the trait:
//!
//! * [`BuiltinEasing`] — closed-form easings (`Linear`, `EaseInOut`,
//!   `SineOut`, `CubicBezier { .. }`, …).
//! * [`KeyframeCurve`] — a sorted list of `(t, v)` pairs plus a
//!   [`KeyframeInterp`] mode.
//!
//! Keyframe curves can also be built by **sampling a user-supplied
//! function** at spec-build time via [`KeyframeCurve::from_curve_fn`] or
//! [`KeyframeCurve::from_motion_fn`]. The callable is invoked only during
//! construction; the resulting curve is pure data and the tick loop never
//! calls back into user code.

use core::f32::consts::{FRAC_PI_2, PI};
use std::collections::HashMap;

use crate::{CurveError, Point};

/// Trait implemented by every curve variant.
///
/// `eval` accepts any `t` and internally saturates it into `[0, 1]`.
pub trait AnimationCurve: Send + Sync + core::fmt::Debug {
    fn eval(&self, t: f32) -> f32;
}

// ---------------------------------------------------------------------------
// BuiltinEasing
// ---------------------------------------------------------------------------

/// Closed-form easing functions.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum BuiltinEasing {
    /// `f(t) = t`
    Linear,

    // Quadratic (aliased to the "ease" names).
    EaseIn,
    EaseOut,
    EaseInOut,
    QuadIn,
    QuadOut,
    QuadInOut,

    // Cubic.
    CubicIn,
    CubicOut,
    CubicInOut,

    // Sinusoidal.
    SineIn,
    SineOut,
    SineInOut,

    /// CSS-style parametric cubic Bézier through `(0,0)`, `(c1x,c1y)`,
    /// `(c2x,c2y)`, `(1,1)`. `c1x` and `c2x` must lie in `[0, 1]` so the
    /// curve is a proper function of `t`.
    CubicBezier { c1x: f32, c1y: f32, c2x: f32, c2y: f32 },
}

impl BuiltinEasing {
    /// Evaluate the curve at `t`.
    pub fn eval(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        use BuiltinEasing::*;
        match *self {
            Linear => t,
            EaseIn | QuadIn => t * t,
            EaseOut | QuadOut => t * (2.0 - t),
            EaseInOut | QuadInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
            CubicIn => t * t * t,
            CubicOut => {
                let u = 1.0 - t;
                1.0 - u * u * u
            }
            CubicInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    let u = -2.0 * t + 2.0;
                    1.0 - u * u * u / 2.0
                }
            }
            SineIn => 1.0 - (t * FRAC_PI_2).cos(),
            SineOut => (t * FRAC_PI_2).sin(),
            SineInOut => -((PI * t).cos() - 1.0) / 2.0,
            CubicBezier { c1x, c1y, c2x, c2y } => cubic_bezier_eval(c1x, c1y, c2x, c2y, t),
        }
    }

    /// Validate parametric variants. Called once at [`Curve`] construction.
    pub fn validate(&self) -> Result<(), CurveError> {
        if let BuiltinEasing::CubicBezier { c1x, c2x, .. } = *self {
            if !c1x.is_finite() || !c2x.is_finite() || !(0.0..=1.0).contains(&c1x)
                || !(0.0..=1.0).contains(&c2x)
            {
                return Err(CurveError::InvalidBezier { c1x, c2x });
            }
        }
        Ok(())
    }
}

/// Solve the cubic Bézier `Y` given a normalized time `x`, using
/// Newton-Raphson with a bisection fallback. Mirrors WebKit's
/// `UnitBezier` implementation.
fn cubic_bezier_eval(c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }

    // Polynomial coefficients for the Bernstein basis with P0 = 0, P3 = 1.
    let cx = 3.0 * c1x;
    let bx = 3.0 * (c2x - c1x) - cx;
    let ax = 1.0 - cx - bx;
    let cy = 3.0 * c1y;
    let by = 3.0 * (c2y - c1y) - cy;
    let ay = 1.0 - cy - by;

    let sample_x = |s: f32| ((ax * s + bx) * s + cx) * s;
    let sample_dx = |s: f32| (3.0 * ax * s + 2.0 * bx) * s + cx;
    let sample_y = |s: f32| ((ay * s + by) * s + cy) * s;

    // Newton-Raphson for a few iterations.
    let mut s = x;
    for _ in 0..8 {
        let x_est = sample_x(s);
        let dx = sample_dx(s);
        if dx.abs() < 1e-6 {
            break;
        }
        let delta = (x_est - x) / dx;
        s -= delta;
        if delta.abs() < 1e-6 {
            return sample_y(s.clamp(0.0, 1.0));
        }
    }

    // Bisection fallback (guaranteed convergence).
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    let mut s = x;
    for _ in 0..32 {
        let x_est = sample_x(s);
        if (x_est - x).abs() < 1e-7 {
            break;
        }
        if x_est < x {
            lo = s;
        } else {
            hi = s;
        }
        s = (lo + hi) * 0.5;
    }
    sample_y(s.clamp(0.0, 1.0))
}

// ---------------------------------------------------------------------------
// Keyframes
// ---------------------------------------------------------------------------

/// One entry in a [`KeyframeCurve`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Keyframe {
    pub t: f32,
    pub v: f32,
}

impl Keyframe {
    #[inline]
    pub const fn new(t: f32, v: f32) -> Self {
        Self { t, v }
    }
}

/// How adjacent keyframes are interpolated.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum KeyframeInterp {
    /// Straight-line interpolation.
    #[default]
    Linear,
    /// Step function — value snaps to the left keyframe until the next one.
    Step,
    /// Hermite smoothstep `s = t² · (3 − 2t)` between each pair of keys.
    SmoothStep,
}

/// A sorted, validated list of `(t, v)` pairs plus an interpolation mode.
///
/// Invariants (enforced at construction):
///
/// * At least 2 keys.
/// * `keys[0].t == 0.0`, `keys.last().t == 1.0`.
/// * All `t`s are strictly ascending and finite.
/// * All `v`s are finite.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyframeCurve {
    keys: Vec<Keyframe>,
    interp: KeyframeInterp,
}

/// Absolute clamp applied to every sampled value in
/// [`KeyframeCurve::from_curve_fn`] / [`KeyframeCurve::from_motion_fn`].
///
/// Values outside `[-VALUE_CLAMP, VALUE_CLAMP]` (or non-finite) are rejected
/// with [`CurveError::SampledValueInvalid`]. The bound is generous enough for
/// spring / bounce / elastic curves while still catching runaway user code.
pub const VALUE_CLAMP: f32 = 100.0;

/// Minimum permissible sample count for a function-generated curve.
pub const MIN_SAMPLES: u32 = 2;
/// Maximum permissible sample count for a function-generated curve.
pub const MAX_SAMPLES: u32 = 4096;

impl KeyframeCurve {
    /// Construct from an explicit list of keys.
    pub fn new(keys: Vec<Keyframe>, interp: KeyframeInterp) -> Result<Self, CurveError> {
        Self::validate(&keys)?;
        Ok(Self { keys, interp })
    }

    /// Borrow the internal key list.
    #[inline]
    pub fn keys(&self) -> &[Keyframe] {
        &self.keys
    }

    #[inline]
    pub fn interpolation(&self) -> KeyframeInterp {
        self.interp
    }

    /// Evaluate the curve at `t`, saturating outside `[0, 1]`.
    pub fn eval(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        // `partition_point` returns the first index whose key.t is strictly
        // greater than `t`. Because keys are strictly ascending it uniquely
        // locates the segment `[idx-1, idx]`.
        let idx = self.keys.partition_point(|k| k.t <= t);
        if idx == 0 {
            return self.keys[0].v;
        }
        if idx >= self.keys.len() {
            return self.keys[self.keys.len() - 1].v;
        }
        let a = self.keys[idx - 1];
        let b = self.keys[idx];
        let span = b.t - a.t;
        if span <= f32::EPSILON {
            return b.v;
        }
        let local = (t - a.t) / span;
        match self.interp {
            KeyframeInterp::Linear => a.v + (b.v - a.v) * local,
            KeyframeInterp::Step => a.v,
            KeyframeInterp::SmoothStep => {
                let s = local * local * (3.0 - 2.0 * local);
                a.v + (b.v - a.v) * s
            }
        }
    }

    /// Build a curve by sampling a plain normalized function
    /// `f(t) -> v` at `samples` evenly-spaced points in `[0, 1]`. Both
    /// endpoints (`t = 0` and `t = 1`) are always sampled.
    ///
    /// The callable is invoked only during this call and dropped before
    /// returning; the resulting curve is pure data.
    ///
    /// Rejects sample counts outside `[MIN_SAMPLES, MAX_SAMPLES]` and any
    /// sample that is non-finite or outside `[-VALUE_CLAMP, VALUE_CLAMP]`.
    pub fn from_curve_fn<F>(
        mut f: F,
        samples: u32,
        interp: KeyframeInterp,
    ) -> Result<Self, CurveError>
    where
        F: FnMut(f32) -> f32,
    {
        Self::sample(samples, interp, |t| f(t))
    }

    /// Build a curve by sampling a **motion-aware** function
    /// `f(&ctx, t) -> v`. The callable receives the movement metadata
    /// (origin, target, distance, duration, user-supplied params) so it
    /// can shape the curve procedurally (spring, gravity, physics …).
    ///
    /// Same constraints and drop semantics as [`Self::from_curve_fn`].
    pub fn from_motion_fn<F>(
        mut f: F,
        ctx: &MotionContext,
        samples: u32,
        interp: KeyframeInterp,
    ) -> Result<Self, CurveError>
    where
        F: FnMut(&MotionContext, f32) -> f32,
    {
        Self::sample(samples, interp, |t| f(ctx, t))
    }

    fn sample<F: FnMut(f32) -> f32>(
        samples: u32,
        interp: KeyframeInterp,
        mut f: F,
    ) -> Result<Self, CurveError> {
        if !(MIN_SAMPLES..=MAX_SAMPLES).contains(&samples) {
            return Err(CurveError::InvalidSampleCount { got: samples });
        }

        let n = samples as usize;
        let mut raw: Vec<Keyframe> = Vec::with_capacity(n);
        let last_i = n - 1;
        for i in 0..n {
            // Force endpoints to exact 0.0 and 1.0 to survive float rounding.
            let t = if i == 0 {
                0.0
            } else if i == last_i {
                1.0
            } else {
                i as f32 / last_i as f32
            };
            let v = f(t);
            if !v.is_finite() || v < -VALUE_CLAMP || v > VALUE_CLAMP {
                return Err(CurveError::SampledValueInvalid { t, v });
            }
            raw.push(Keyframe { t, v });
        }

        // Collapse runs of identical values. Preserve the first and last
        // key of every plateau so the curve shape is unchanged.
        let mut dedup: Vec<Keyframe> = Vec::with_capacity(raw.len());
        for (i, k) in raw.iter().enumerate() {
            if i == 0 || i == last_i {
                dedup.push(*k);
                continue;
            }
            let prev_v = raw[i - 1].v;
            let next_v = raw[i + 1].v;
            if prev_v == k.v && next_v == k.v {
                continue;
            }
            dedup.push(*k);
        }

        Self::new(dedup, interp)
    }

    fn validate(keys: &[Keyframe]) -> Result<(), CurveError> {
        if keys.len() < 2 {
            return Err(CurveError::TooFewKeys(keys.len()));
        }
        if keys[0].t != 0.0 {
            return Err(CurveError::FirstKeyNotZero(keys[0].t));
        }
        let last = keys[keys.len() - 1];
        if last.t != 1.0 {
            return Err(CurveError::LastKeyNotOne(last.t));
        }
        let mut prev_t = f32::NEG_INFINITY;
        for (i, k) in keys.iter().enumerate() {
            if !k.t.is_finite() || k.t < 0.0 || k.t > 1.0 || k.t <= prev_t {
                // Special-case the first key: prev_t is -inf so t=0 is fine.
                if !(i == 0 && k.t == 0.0) {
                    return Err(CurveError::InvalidKeyOrder { index: i, t: k.t });
                }
            }
            if !k.v.is_finite() {
                return Err(CurveError::NonFiniteValue {
                    index: i,
                    t: k.t,
                    v: k.v,
                });
            }
            prev_t = k.t;
        }
        Ok(())
    }
}

impl AnimationCurve for KeyframeCurve {
    #[inline]
    fn eval(&self, t: f32) -> f32 {
        self.eval(t)
    }
}

// ---------------------------------------------------------------------------
// MotionContext
// ---------------------------------------------------------------------------

/// Movement metadata handed to motion-aware curve samplers.
#[derive(Clone, Debug)]
pub struct MotionContext {
    pub origin: Point,
    pub target: Point,
    /// Euclidean distance between `origin` and `target`, in pixels.
    pub distance_px: f32,
    /// Concrete wall-clock duration the animation will run for.
    pub duration: std::time::Duration,
    /// Free-form parameter bag for the sampler function.
    pub params: HashMap<String, f64>,
}

impl MotionContext {
    /// Construct with `distance_px` computed automatically.
    pub fn new(origin: Point, target: Point, duration: std::time::Duration) -> Self {
        Self {
            origin,
            target,
            distance_px: Point::distance(origin, target),
            duration,
            params: HashMap::new(),
        }
    }

    #[must_use]
    pub fn with_params(mut self, params: HashMap<String, f64>) -> Self {
        self.params = params;
        self
    }

    #[must_use]
    pub fn with_param(mut self, key: impl Into<String>, value: f64) -> Self {
        self.params.insert(key.into(), value);
        self
    }

    #[inline]
    pub fn param(&self, key: &str) -> Option<f64> {
        self.params.get(key).copied()
    }
}

// ---------------------------------------------------------------------------
// Curve
// ---------------------------------------------------------------------------

/// The public curve type used inside every [`crate::IconAnimationSpec`].
///
/// It combines the two variants under one enum so specs can be cloned and
/// stored without dynamic dispatch on the hot path.
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    Builtin(BuiltinEasing),
    Keyframe(KeyframeCurve),
}

impl Curve {
    // --- Builtin constructors ---------------------------------------------

    pub fn linear() -> Self {
        Self::Builtin(BuiltinEasing::Linear)
    }
    pub fn ease_in() -> Self {
        Self::Builtin(BuiltinEasing::EaseIn)
    }
    pub fn ease_out() -> Self {
        Self::Builtin(BuiltinEasing::EaseOut)
    }
    pub fn ease_in_out() -> Self {
        Self::Builtin(BuiltinEasing::EaseInOut)
    }
    pub fn quad_in() -> Self {
        Self::Builtin(BuiltinEasing::QuadIn)
    }
    pub fn quad_out() -> Self {
        Self::Builtin(BuiltinEasing::QuadOut)
    }
    pub fn quad_in_out() -> Self {
        Self::Builtin(BuiltinEasing::QuadInOut)
    }
    pub fn cubic_in() -> Self {
        Self::Builtin(BuiltinEasing::CubicIn)
    }
    pub fn cubic_out() -> Self {
        Self::Builtin(BuiltinEasing::CubicOut)
    }
    pub fn cubic_in_out() -> Self {
        Self::Builtin(BuiltinEasing::CubicInOut)
    }
    pub fn sine_in() -> Self {
        Self::Builtin(BuiltinEasing::SineIn)
    }
    pub fn sine_out() -> Self {
        Self::Builtin(BuiltinEasing::SineOut)
    }
    pub fn sine_in_out() -> Self {
        Self::Builtin(BuiltinEasing::SineInOut)
    }

    /// Parametric cubic Bézier easing. `c1x` and `c2x` must lie in `[0, 1]`.
    pub fn cubic_bezier(c1x: f32, c1y: f32, c2x: f32, c2y: f32) -> Result<Self, CurveError> {
        let e = BuiltinEasing::CubicBezier { c1x, c1y, c2x, c2y };
        e.validate()?;
        Ok(Self::Builtin(e))
    }

    // --- Keyframe constructors --------------------------------------------

    pub fn keyframes(keys: Vec<Keyframe>, interp: KeyframeInterp) -> Result<Self, CurveError> {
        Ok(Self::Keyframe(KeyframeCurve::new(keys, interp)?))
    }

    pub fn from_curve_fn<F>(f: F, samples: u32, interp: KeyframeInterp) -> Result<Self, CurveError>
    where
        F: FnMut(f32) -> f32,
    {
        Ok(Self::Keyframe(KeyframeCurve::from_curve_fn(
            f, samples, interp,
        )?))
    }

    pub fn from_motion_fn<F>(
        f: F,
        ctx: &MotionContext,
        samples: u32,
        interp: KeyframeInterp,
    ) -> Result<Self, CurveError>
    where
        F: FnMut(&MotionContext, f32) -> f32,
    {
        Ok(Self::Keyframe(KeyframeCurve::from_motion_fn(
            f, ctx, samples, interp,
        )?))
    }
}

impl AnimationCurve for Curve {
    #[inline]
    fn eval(&self, t: f32) -> f32 {
        match self {
            Curve::Builtin(e) => e.eval(t),
            Curve::Keyframe(k) => KeyframeCurve::eval(k, t),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn near(a: f32, b: f32) {
        assert!((a - b).abs() < EPS, "expected ~{b}, got {a}");
    }

    // --- BuiltinEasing ----------------------------------------------------

    #[test]
    fn builtin_endpoints_for_non_overshoot() {
        for e in [
            BuiltinEasing::Linear,
            BuiltinEasing::EaseIn,
            BuiltinEasing::EaseOut,
            BuiltinEasing::EaseInOut,
            BuiltinEasing::QuadIn,
            BuiltinEasing::QuadOut,
            BuiltinEasing::QuadInOut,
            BuiltinEasing::CubicIn,
            BuiltinEasing::CubicOut,
            BuiltinEasing::CubicInOut,
            BuiltinEasing::SineIn,
            BuiltinEasing::SineOut,
            BuiltinEasing::SineInOut,
        ] {
            near(e.eval(0.0), 0.0);
            near(e.eval(1.0), 1.0);
        }
    }

    #[test]
    fn linear_is_identity() {
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            near(BuiltinEasing::Linear.eval(t), t);
        }
    }

    #[test]
    fn builtin_saturates_outside_unit_interval() {
        near(BuiltinEasing::CubicInOut.eval(-5.0), 0.0);
        near(BuiltinEasing::CubicInOut.eval(5.0), 1.0);
    }

    #[test]
    fn cubic_bezier_matches_linear_for_default_line() {
        // The Bezier (0,0)-(1/3,1/3)-(2/3,2/3)-(1,1) is a straight line.
        let c = Curve::cubic_bezier(1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0).unwrap();
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            near(c.eval(t), t);
        }
    }

    #[test]
    fn cubic_bezier_rejects_out_of_range_x() {
        let err = Curve::cubic_bezier(-0.1, 0.0, 1.1, 0.0).unwrap_err();
        assert!(matches!(err, CurveError::InvalidBezier { .. }));
    }

    // --- KeyframeCurve ----------------------------------------------------

    #[test]
    fn keyframe_rejects_too_few_keys() {
        let err = KeyframeCurve::new(vec![Keyframe::new(0.0, 0.0)], KeyframeInterp::Linear)
            .unwrap_err();
        assert!(matches!(err, CurveError::TooFewKeys(1)));
    }

    #[test]
    fn keyframe_rejects_bad_endpoints() {
        let err = KeyframeCurve::new(
            vec![Keyframe::new(0.1, 0.0), Keyframe::new(1.0, 1.0)],
            KeyframeInterp::Linear,
        )
        .unwrap_err();
        assert!(matches!(err, CurveError::FirstKeyNotZero(_)));

        let err = KeyframeCurve::new(
            vec![Keyframe::new(0.0, 0.0), Keyframe::new(0.9, 1.0)],
            KeyframeInterp::Linear,
        )
        .unwrap_err();
        assert!(matches!(err, CurveError::LastKeyNotOne(_)));
    }

    #[test]
    fn keyframe_rejects_non_ascending_times() {
        let err = KeyframeCurve::new(
            vec![
                Keyframe::new(0.0, 0.0),
                Keyframe::new(0.5, 0.5),
                Keyframe::new(0.4, 0.4),
                Keyframe::new(1.0, 1.0),
            ],
            KeyframeInterp::Linear,
        )
        .unwrap_err();
        assert!(matches!(err, CurveError::InvalidKeyOrder { .. }));
    }

    #[test]
    fn keyframe_rejects_nan_value() {
        let err = KeyframeCurve::new(
            vec![
                Keyframe::new(0.0, 0.0),
                Keyframe::new(0.5, f32::NAN),
                Keyframe::new(1.0, 1.0),
            ],
            KeyframeInterp::Linear,
        )
        .unwrap_err();
        assert!(matches!(err, CurveError::NonFiniteValue { .. }));
    }

    #[test]
    fn keyframe_linear_interpolates() {
        let c = KeyframeCurve::new(
            vec![
                Keyframe::new(0.0, 0.0),
                Keyframe::new(0.5, 0.2),
                Keyframe::new(1.0, 1.0),
            ],
            KeyframeInterp::Linear,
        )
        .unwrap();
        near(c.eval(0.0), 0.0);
        near(c.eval(0.25), 0.1);
        near(c.eval(0.5), 0.2);
        near(c.eval(0.75), 0.6);
        near(c.eval(1.0), 1.0);
    }

    #[test]
    fn keyframe_step_interpolates() {
        let c = KeyframeCurve::new(
            vec![
                Keyframe::new(0.0, 0.0),
                Keyframe::new(0.5, 1.0),
                Keyframe::new(1.0, 1.0),
            ],
            KeyframeInterp::Step,
        )
        .unwrap();
        near(c.eval(0.49), 0.0);
        near(c.eval(0.5), 1.0);
        near(c.eval(0.99), 1.0);
    }

    #[test]
    fn keyframe_smoothstep_matches_hermite_midpoint() {
        let c = KeyframeCurve::new(
            vec![Keyframe::new(0.0, 0.0), Keyframe::new(1.0, 1.0)],
            KeyframeInterp::SmoothStep,
        )
        .unwrap();
        // smoothstep(0.5) = 0.5.
        near(c.eval(0.5), 0.5);
        // smoothstep(0.25) = 0.25² · (3 − 2·0.25) = 0.15625.
        near(c.eval(0.25), 0.15625);
    }

    // --- from_curve_fn / from_motion_fn -----------------------------------

    #[test]
    fn from_curve_fn_rejects_bad_sample_counts() {
        for &bad in &[0u32, 1, MAX_SAMPLES + 1, u32::MAX] {
            let err =
                KeyframeCurve::from_curve_fn(|t| t, bad, KeyframeInterp::Linear).unwrap_err();
            assert!(matches!(err, CurveError::InvalidSampleCount { .. }), "sample {bad}");
        }
    }

    #[test]
    fn from_curve_fn_samples_linear_identity() {
        let c = KeyframeCurve::from_curve_fn(|t| t, 33, KeyframeInterp::Linear).unwrap();
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            near(c.eval(t), t);
        }
    }

    #[test]
    fn from_curve_fn_rejects_nan_or_infinite() {
        let err = KeyframeCurve::from_curve_fn(|_| f32::NAN, 8, KeyframeInterp::Linear)
            .unwrap_err();
        assert!(matches!(err, CurveError::SampledValueInvalid { .. }));

        let err = KeyframeCurve::from_curve_fn(|_| f32::INFINITY, 8, KeyframeInterp::Linear)
            .unwrap_err();
        assert!(matches!(err, CurveError::SampledValueInvalid { .. }));
    }

    #[test]
    fn from_curve_fn_rejects_out_of_clamp() {
        let err =
            KeyframeCurve::from_curve_fn(|_| 200.0, 8, KeyframeInterp::Linear).unwrap_err();
        assert!(matches!(err, CurveError::SampledValueInvalid { t: _, v }
                          if (v - 200.0).abs() < 1e-6));
    }

    #[test]
    fn from_curve_fn_only_calls_callable_at_build_time() {
        let count = std::sync::atomic::AtomicUsize::new(0);
        let curve = KeyframeCurve::from_curve_fn(
            |t| {
                count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                t
            },
            10,
            KeyframeInterp::Linear,
        )
        .unwrap();
        let calls_after_build = count.load(std::sync::atomic::Ordering::Relaxed);
        // Sample the curve many times; the underlying closure must never
        // be re-invoked.
        for _ in 0..1_000 {
            let _ = curve.eval(0.42);
        }
        assert_eq!(count.load(std::sync::atomic::Ordering::Relaxed), calls_after_build);
        assert_eq!(calls_after_build, 10);
    }

    #[test]
    fn from_curve_fn_deduplicates_flat_middle() {
        // Constant 0.5 in the middle: expect the two edge keys to survive
        // plus at least one entry per plateau boundary.
        let c = KeyframeCurve::from_curve_fn(
            |t| if t < 0.25 || t > 0.75 { t } else { 0.5 },
            33,
            KeyframeInterp::Linear,
        )
        .unwrap();
        // Should be fewer than 33 keys after dedup.
        assert!(c.keys().len() < 33, "dedup left {} keys", c.keys().len());
        near(c.eval(0.0), 0.0);
        near(c.eval(1.0), 1.0);
    }

    #[test]
    fn from_motion_fn_receives_context() {
        let ctx = MotionContext::new(
            Point::new(0, 0),
            Point::new(300, 400),
            std::time::Duration::from_secs(1),
        )
        .with_param("g", 9.81);
        assert!((ctx.distance_px - 500.0).abs() < 1e-4);
        assert_eq!(ctx.param("g"), Some(9.81));

        let curve = KeyframeCurve::from_motion_fn(
            |ctx, t| {
                // Simple: scale t by params["g"]/10.
                let g = ctx.params.get("g").copied().unwrap_or(1.0) as f32;
                (t * g / 10.0).min(1.0)
            },
            &ctx,
            16,
            KeyframeInterp::Linear,
        )
        .unwrap();
        // At t=1 the raw value is 0.981 (< 1) → curve endpoint stays 0.981.
        assert!(curve.eval(1.0) > 0.9);
    }

    // --- Curve wrapping ---------------------------------------------------

    #[test]
    fn curve_dispatch_matches_underlying_variant() {
        let a = Curve::linear();
        let b = Curve::from_curve_fn(|t| t, 8, KeyframeInterp::Linear).unwrap();
        near(a.eval(0.42), 0.42);
        near(b.eval(0.42), 0.42);
    }

    // --- Send/Sync -------------------------------------------------------

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn curves_are_send_sync() {
        assert_send_sync::<Curve>();
        assert_send_sync::<KeyframeCurve>();
        assert_send_sync::<BuiltinEasing>();
    }
}

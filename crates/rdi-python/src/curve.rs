//! Python wrapper for [`rdi_core::Curve`].
//!
//! Curves are opaque, immutable objects. Every constructor is a
//! classmethod; the underlying enum is inaccessible from Python, so the
//! representation can evolve without a breaking change.
//!
//! # Function-generated curves
//!
//! [`PyCurve::from_function`] and [`PyCurve::from_motion_function`]
//! sample a user-supplied Python callable **once** at construction time
//! (under the GIL) and store only the resulting keyframe data. The tick
//! loop never calls back into Python.

use std::collections::HashMap;
use std::time::Duration as StdDuration;

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString, PyTuple, PyType};

use rdi_core::{
    procedural, Curve as CoreCurve, KeyframeInterp, MotionContext, Point,
};

use crate::errors::map_curve_error;
use crate::geometry::point_from_any;

// ---------------------------------------------------------------------------
// Interp parsing
// ---------------------------------------------------------------------------

/// Parse a Python string into a [`KeyframeInterp`]. Accepts `"linear"`,
/// `"step"`, `"smoothstep"` and `"smooth_step"` (case-insensitive).
fn parse_interp(s: &str) -> PyResult<KeyframeInterp> {
    match s.to_ascii_lowercase().as_str() {
        "linear" => Ok(KeyframeInterp::Linear),
        "step" => Ok(KeyframeInterp::Step),
        "smoothstep" | "smooth_step" => Ok(KeyframeInterp::SmoothStep),
        other => Err(PyValueError::new_err(format!(
            "unknown interpolation mode: {other:?} (expected 'linear', 'step', or 'smoothstep')"
        ))),
    }
}

// ---------------------------------------------------------------------------
// PyCurve
// ---------------------------------------------------------------------------

/// Animation curve.
///
/// Every axis of every icon animation is driven by a value in `[0, 1]`
/// produced by evaluating a `Curve`. Overshoot (values outside `[0, 1]`)
/// is permitted.
#[pyclass(frozen, from_py_object, module = "rusty_desktop_icons", name = "Curve")]
#[derive(Clone, Debug)]
pub struct PyCurve {
    pub(crate) inner: CoreCurve,
}

impl PyCurve {
    pub(crate) fn from_inner(inner: CoreCurve) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyCurve {
    // ---- Builtin easings ------------------------------------------------

    #[classmethod]
    fn linear(_cls: &Bound<'_, PyType>) -> Self {
        Self::from_inner(CoreCurve::linear())
    }
    #[classmethod]
    fn ease_in(_cls: &Bound<'_, PyType>) -> Self {
        Self::from_inner(CoreCurve::ease_in())
    }
    #[classmethod]
    fn ease_out(_cls: &Bound<'_, PyType>) -> Self {
        Self::from_inner(CoreCurve::ease_out())
    }
    #[classmethod]
    fn ease_in_out(_cls: &Bound<'_, PyType>) -> Self {
        Self::from_inner(CoreCurve::ease_in_out())
    }
    #[classmethod]
    fn quad_in(_cls: &Bound<'_, PyType>) -> Self {
        Self::from_inner(CoreCurve::quad_in())
    }
    #[classmethod]
    fn quad_out(_cls: &Bound<'_, PyType>) -> Self {
        Self::from_inner(CoreCurve::quad_out())
    }
    #[classmethod]
    fn quad_in_out(_cls: &Bound<'_, PyType>) -> Self {
        Self::from_inner(CoreCurve::quad_in_out())
    }
    #[classmethod]
    fn cubic_in(_cls: &Bound<'_, PyType>) -> Self {
        Self::from_inner(CoreCurve::cubic_in())
    }
    #[classmethod]
    fn cubic_out(_cls: &Bound<'_, PyType>) -> Self {
        Self::from_inner(CoreCurve::cubic_out())
    }
    #[classmethod]
    fn cubic_in_out(_cls: &Bound<'_, PyType>) -> Self {
        Self::from_inner(CoreCurve::cubic_in_out())
    }
    #[classmethod]
    fn sine_in(_cls: &Bound<'_, PyType>) -> Self {
        Self::from_inner(CoreCurve::sine_in())
    }
    #[classmethod]
    fn sine_out(_cls: &Bound<'_, PyType>) -> Self {
        Self::from_inner(CoreCurve::sine_out())
    }
    #[classmethod]
    fn sine_in_out(_cls: &Bound<'_, PyType>) -> Self {
        Self::from_inner(CoreCurve::sine_in_out())
    }

    /// CSS-style parametric cubic Bézier through `(0, 0)`, `(c1x, c1y)`,
    /// `(c2x, c2y)` and `(1, 1)`. `c1x` and `c2x` must lie in `[0, 1]`.
    #[classmethod]
    #[pyo3(signature = (c1x, c1y, c2x, c2y))]
    fn cubic_bezier(
        _cls: &Bound<'_, PyType>,
        c1x: f32,
        c1y: f32,
        c2x: f32,
        c2y: f32,
    ) -> PyResult<Self> {
        CoreCurve::cubic_bezier(c1x, c1y, c2x, c2y)
            .map(Self::from_inner)
            .map_err(map_curve_error)
    }

    // ---- Keyframes ------------------------------------------------------

    /// Build a curve from an explicit list of `(t, v)` keyframes.
    ///
    /// `keys` must start with `(0.0, ...)`, end with `(1.0, ...)`, and be
    /// strictly ascending in `t`. Prefer `KeyframeInterp` members for `interp`;
    /// legacy strings and case-insensitive aliases remain accepted.
    #[classmethod]
    #[pyo3(signature = (keys: "list[tuple[float, float] | list[float] | dict[str, float]]", interp: "KeyframeInterp | str"="linear"))]
    fn keyframes(
        _cls: &Bound<'_, PyType>,
        keys: &Bound<'_, PyAny>,
        interp: &str,
    ) -> PyResult<Self> {
        let interp = parse_interp(interp)?;
        let raw = extract_keys(keys)?;
        CoreCurve::keyframes(raw, interp)
            .map(Self::from_inner)
            .map_err(map_curve_error)
    }

    /// Sample a Python callable `f(t) -> v` at `samples` evenly-spaced
    /// points in `[0, 1]` and build a keyframe curve.
    ///
    /// The callable is invoked **only** during this call and dropped
    /// immediately after; the resulting curve is pure data. Any exception
    /// raised by the callable becomes an `InvalidCurve` error.
    #[classmethod]
    #[pyo3(signature = (f: "typing.Callable[[float], float]", samples=64, interp: "KeyframeInterp | str"="linear"))]
    fn from_function(
        _cls: &Bound<'_, PyType>,
        py: Python<'_>,
        f: Py<PyAny>,
        samples: u32,
        interp: &str,
    ) -> PyResult<Self> {
        let interp = parse_interp(interp)?;

        let mut sampling_err: Option<PyErr> = None;
        let result = CoreCurve::from_curve_fn(
            |t| {
                if sampling_err.is_some() {
                    return 0.0;
                }
                match f.call1(py, (t as f64,)).and_then(|v| v.extract::<f64>(py)) {
                    Ok(v) => v as f32,
                    Err(e) => {
                        sampling_err = Some(e);
                        0.0
                    }
                }
            },
            samples,
            interp,
        );

        if let Some(e) = sampling_err {
            return Err(e);
        }
        result.map(Self::from_inner).map_err(map_curve_error)
    }

    /// Sample a **motion-aware** Python callable `f(ctx, t) -> v` at
    /// `samples` evenly-spaced points in `[0, 1]`.
    ///
    /// The callable receives a
    /// [`MotionContext`](rusty_desktop_icons.MotionContext) built from
    /// `origin`, `target`, `duration_seconds` and `params`, plus the
    /// normalized time `t`. It is invoked only during this call.
    #[classmethod]
    #[pyo3(signature = (
        f: "typing.Callable[[MotionContext, float], float]",
        origin: "typing.Sequence[int]",
        target: "typing.Sequence[int]",
        duration_seconds,
        params=None,
        samples=64,
        interp: "KeyframeInterp | str"="linear",
    ))]
    #[allow(clippy::too_many_arguments)]
    fn from_motion_function(
        _cls: &Bound<'_, PyType>,
        py: Python<'_>,
        f: Py<PyAny>,
        origin: &Bound<'_, PyAny>,
        target: &Bound<'_, PyAny>,
        duration_seconds: f64,
        params: Option<HashMap<String, f64>>,
        samples: u32,
        interp: &str,
    ) -> PyResult<Self> {
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
            return Err(PyValueError::new_err(format!(
                "duration_seconds must be a positive finite number, got {duration_seconds}"
            )));
        }
        let interp = parse_interp(interp)?;
        let origin_pt = point_from_any(origin)?;
        let target_pt = point_from_any(target)?;
        let duration = StdDuration::try_from_secs_f64(duration_seconds).map_err(|e| {
            PyValueError::new_err(format!(
                "duration_seconds could not be represented as a std::time::Duration: {e}"
            ))
        })?;

        let mut ctx = MotionContext::new(origin_pt, target_pt, duration);
        if let Some(p) = params {
            ctx.params = p;
        }

        // Build a Python-visible ctx object once so the callable receives
        // a consistent snapshot for every sample.
        let py_ctx: Py<PyAny> = Py::new(py, PyMotionContext::from_core(&ctx))?.into_any();

        let mut sampling_err: Option<PyErr> = None;
        let result = CoreCurve::from_motion_fn(
            |_ctx_ref, t| {
                if sampling_err.is_some() {
                    return 0.0;
                }
                match f
                    .call1(py, (py_ctx.clone_ref(py), t as f64))
                    .and_then(|v| v.extract::<f64>(py))
                {
                    Ok(v) => v as f32,
                    Err(e) => {
                        sampling_err = Some(e);
                        0.0
                    }
                }
            },
            &ctx,
            samples,
            interp,
        );

        if let Some(e) = sampling_err {
            return Err(e);
        }
        result.map(Self::from_inner).map_err(map_curve_error)
    }

    // ---- Procedural helpers ---------------------------------------------

    /// Damped-cosine "spring" curve.
    #[classmethod]
    #[pyo3(signature = (damping=0.5, stiffness=180.0, mass=1.0))]
    fn spring(
        _cls: &Bound<'_, PyType>,
        damping: f32,
        stiffness: f32,
        mass: f32,
    ) -> PyResult<Self> {
        procedural::spring(damping, stiffness, mass)
            .map(Self::from_inner)
            .map_err(map_curve_error)
    }

    /// Bouncing curve — several damped rebounds converging on `1`.
    #[classmethod]
    #[pyo3(signature = (bounces=3, decay=0.5))]
    fn bounce(_cls: &Bound<'_, PyType>, bounces: u32, decay: f32) -> PyResult<Self> {
        procedural::bounce(bounces, decay)
            .map(Self::from_inner)
            .map_err(map_curve_error)
    }

    /// Elastic ease-out — sinusoidal overshoot damped by `2^(-10t)`.
    #[classmethod]
    #[pyo3(signature = (period=0.3, amplitude=1.0))]
    fn elastic(_cls: &Bound<'_, PyType>, period: f32, amplitude: f32) -> PyResult<Self> {
        procedural::elastic(period, amplitude)
            .map(Self::from_inner)
            .map_err(map_curve_error)
    }

    /// Back / overshoot ease-in-out.
    #[classmethod]
    #[pyo3(signature = (strength=1.7))]
    fn overshoot(_cls: &Bound<'_, PyType>, strength: f32) -> PyResult<Self> {
        procedural::overshoot(strength)
            .map(Self::from_inner)
            .map_err(map_curve_error)
    }

    // ---- Introspection --------------------------------------------------

    /// Curve representation, compatible with the legacy string discriminator.
    #[getter]
    fn kind(&self) -> crate::enums::CurveKind {
        let value = match self.inner {
            CoreCurve::Builtin(_) => "builtin",
            CoreCurve::Keyframe(_) => "keyframe",
        };
        crate::enums::CurveKind(value)
    }

    /// Evaluate the curve at `t` (saturated to `[0, 1]`).
    fn eval(&self, t: f32) -> f32 {
        use rdi_core::AnimationCurve;
        self.inner.eval(t)
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            CoreCurve::Builtin(e) => format!("Curve({e:?})"),
            CoreCurve::Keyframe(k) => format!(
                "Curve(keyframe, keys={}, interp={:?})",
                k.keys().len(),
                k.interpolation()
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Motion context (exposed to Python-side samplers)
// ---------------------------------------------------------------------------

/// Read-only view of the motion metadata handed to a
/// [`Curve.from_motion_function`] sampler.
#[pyclass(frozen, skip_from_py_object, module = "rusty_desktop_icons", name = "MotionContext")]
#[derive(Clone, Debug)]
pub struct PyMotionContext {
    origin: Point,
    target: Point,
    #[pyo3(get)]
    distance_px: f32,
    #[pyo3(get)]
    duration_seconds: f64,
    #[pyo3(get)]
    params: HashMap<String, f64>,
}

impl PyMotionContext {
    pub(crate) fn from_core(ctx: &MotionContext) -> Self {
        Self {
            origin: ctx.origin,
            target: ctx.target,
            distance_px: ctx.distance_px,
            duration_seconds: ctx.duration.as_secs_f64(),
            params: ctx.params.clone(),
        }
    }
}

#[pymethods]
impl PyMotionContext {
    #[getter]
    fn origin(&self) -> (i32, i32) {
        (self.origin.x, self.origin.y)
    }

    #[getter]
    fn target(&self) -> (i32, i32) {
        (self.target.x, self.target.y)
    }

    /// Return `params[key]` or the provided default.
    #[pyo3(signature = (key, default=None))]
    fn param(&self, key: &str, default: Option<f64>) -> Option<f64> {
        self.params.get(key).copied().or(default)
    }

    fn __repr__(&self) -> String {
        format!(
            "MotionContext(origin=({}, {}), target=({}, {}), distance_px={:.2}, duration_seconds={:.3}, params={})",
            self.origin.x,
            self.origin.y,
            self.target.x,
            self.target.y,
            self.distance_px,
            self.duration_seconds,
            self.params.len()
        )
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a Python object into a `Vec<rdi_core::Keyframe>`.
///
/// Accepts a list of `(t, v)` tuples, a list of `{"t": ..., "v": ...}`
/// dicts, or a list of 2-element sequences.
fn extract_keys(obj: &Bound<'_, PyAny>) -> PyResult<Vec<rdi_core::Keyframe>> {
    let list: Bound<'_, PyList> = obj.extract().map_err(|_| {
        PyValueError::new_err("keys must be a list of (t, v) pairs or {'t':..,'v':..} dicts")
    })?;
    let mut out = Vec::with_capacity(list.len());
    for (i, item) in list.iter().enumerate() {
        let key = extract_one_key(&item).map_err(|e| {
            PyValueError::new_err(format!("invalid keyframe at index {i}: {e}"))
        })?;
        out.push(key);
    }
    if out.is_empty() {
        return Err(PyValueError::new_err(
            "keys must contain at least 2 entries",
        ));
    }
    Ok(out)
}

fn extract_one_key(item: &Bound<'_, PyAny>) -> PyResult<rdi_core::Keyframe> {
    // Tuple / list.
    if let Ok(tup) = item.cast::<PyTuple>() {
        if tup.len() == 2 {
            let t: f32 = tup.get_item(0)?.extract()?;
            let v: f32 = tup.get_item(1)?.extract()?;
            return Ok(rdi_core::Keyframe::new(t, v));
        }
    }
    if let Ok(list) = item.cast::<PyList>() {
        if list.len() == 2 {
            let t: f32 = list.get_item(0)?.extract()?;
            let v: f32 = list.get_item(1)?.extract()?;
            return Ok(rdi_core::Keyframe::new(t, v));
        }
    }
    // Dict.
    if let Ok(dict) = item.cast::<PyDict>() {
        let t: f32 = dict
            .get_item("t")?
            .ok_or_else(|| PyRuntimeError::new_err("missing 't' key"))?
            .extract()?;
        let v: f32 = dict
            .get_item("v")?
            .ok_or_else(|| PyRuntimeError::new_err("missing 'v' key"))?
            .extract()?;
        return Ok(rdi_core::Keyframe::new(t, v));
    }
    // String is _not_ a valid keyframe, but downcast<PyString> catches it
    // early with a better message.
    if item.cast::<PyString>().is_ok() {
        return Err(PyValueError::new_err(
            "keyframe cannot be a string; expected a 2-element (t, v) tuple",
        ));
    }
    Err(PyValueError::new_err(
        "expected a 2-element (t, v) tuple/list or a {'t':..,'v':..} dict",
    ))
}

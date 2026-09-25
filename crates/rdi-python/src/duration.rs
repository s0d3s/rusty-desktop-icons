//! Python wrapper for [`rdi_core::Duration`].

use std::time::Duration as StdDuration;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use rdi_core::Duration as CoreDuration;

/// Duration policy for an animation.
///
/// Build with one of the classmethod constructors::
///
///     Duration.fixed(seconds=1.0)
///     Duration.distance(speed_px_per_sec=250.0)
///     Duration.distance_clamped(
///         speed_px_per_sec=250.0,
///         min_seconds=0.15,
///         max_seconds=1.5,
///     )
#[pyclass(frozen, from_py_object, module = "rusty_desktop_icons", name = "Duration")]
#[derive(Clone, Debug)]
pub struct PyDuration {
    pub(crate) inner: CoreDuration,
}

impl PyDuration {
    pub(crate) fn from_inner(inner: CoreDuration) -> Self {
        Self { inner }
    }
}

fn secs_to_std(name: &str, seconds: f64) -> PyResult<StdDuration> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(PyValueError::new_err(format!(
            "{name} must be a non-negative finite number, got {seconds}"
        )));
    }
    // The negative / non-finite cases are handled above; the fallible
    // call is kept for the u64 overflow guard.
    StdDuration::try_from_secs_f64(seconds).map_err(|e| {
        PyValueError::new_err(format!(
            "{name} could not be represented as a std::time::Duration: {e}"
        ))
    })
}

#[pymethods]
impl PyDuration {
    /// A fixed wall-clock duration.
    #[classmethod]
    #[pyo3(signature = (seconds))]
    fn fixed(_cls: &Bound<'_, pyo3::types::PyType>, seconds: f64) -> PyResult<Self> {
        let d = secs_to_std("seconds", seconds)?;
        if d.is_zero() {
            return Err(PyValueError::new_err(
                "Duration.fixed(seconds=...) must be strictly greater than zero",
            ));
        }
        Ok(Self::from_inner(CoreDuration::fixed(d)))
    }

    /// Duration derived from movement distance and a constant speed.
    #[classmethod]
    #[pyo3(signature = (speed_px_per_sec))]
    fn distance(
        _cls: &Bound<'_, pyo3::types::PyType>,
        speed_px_per_sec: f32,
    ) -> PyResult<Self> {
        if !speed_px_per_sec.is_finite() || speed_px_per_sec <= 0.0 {
            return Err(PyValueError::new_err(format!(
                "Duration.distance requires a positive finite speed, got {speed_px_per_sec}"
            )));
        }
        Ok(Self::from_inner(CoreDuration::distance(speed_px_per_sec)))
    }

    /// Duration derived from distance and speed, clamped to
    /// `[min_seconds, max_seconds]`.
    #[classmethod]
    #[pyo3(signature = (speed_px_per_sec, min_seconds, max_seconds))]
    fn distance_clamped(
        _cls: &Bound<'_, pyo3::types::PyType>,
        speed_px_per_sec: f32,
        min_seconds: f64,
        max_seconds: f64,
    ) -> PyResult<Self> {
        if !speed_px_per_sec.is_finite() || speed_px_per_sec <= 0.0 {
            return Err(PyValueError::new_err(format!(
                "Duration.distance_clamped requires a positive finite speed, got {speed_px_per_sec}"
            )));
        }
        let lo = secs_to_std("min_seconds", min_seconds)?;
        let hi = secs_to_std("max_seconds", max_seconds)?;
        if lo > hi {
            return Err(PyValueError::new_err(format!(
                "Duration.distance_clamped: min_seconds ({min_seconds}) must be <= max_seconds ({max_seconds})"
            )));
        }
        Ok(Self::from_inner(CoreDuration::distance_clamped(
            speed_px_per_sec,
            lo,
            hi,
        )))
    }

    /// Duration representation, compatible with the legacy string discriminator.
    #[getter]
    fn kind(&self) -> crate::enums::DurationKind {
        let value = match self.inner {
            CoreDuration::Fixed(_) => "fixed",
            CoreDuration::Distance { .. } => "distance",
        };
        crate::enums::DurationKind(value)
    }

    fn __repr__(&self) -> String {
        match self.inner {
            CoreDuration::Fixed(d) => {
                format!("Duration.fixed(seconds={:.3})", d.as_secs_f64())
            }
            CoreDuration::Distance {
                speed_px_per_sec,
                min,
                max,
            } => match (min, max) {
                (Some(lo), Some(hi)) => format!(
                    "Duration.distance_clamped(speed_px_per_sec={}, min_seconds={:.3}, max_seconds={:.3})",
                    speed_px_per_sec,
                    lo.as_secs_f64(),
                    hi.as_secs_f64()
                ),
                _ => format!("Duration.distance(speed_px_per_sec={speed_px_per_sec})"),
            },
        }
    }
}

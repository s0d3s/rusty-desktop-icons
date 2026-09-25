//! Python wrapper for [`rdi_core::AnimationHandle`].
//!
//! Observer callbacks (`on_start`, `on_tick`, `on_icon_complete`,
//! `on_finish`) accept any Python callable. When the worker fires the
//! corresponding event the GIL is re-acquired, the payload wrapped in a
//! `#[pyclass(frozen)]` type and the callable invoked synchronously.
//! Exceptions raised by the callback are swallowed (Python prints them
//! via `sys.unraisablehook`) so a misbehaving observer cannot crash the
//! worker thread.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use rdi_core::AnimationHandle as CoreHandle;

use crate::geometry::{
    PyFinalCommitOutcome, PyFinishReason, PyIconAnimationState, PyStartContext,
    PyStopMode, PyTickContext,
};

/// Handle returned by [`DesktopController.animate`].
///
/// Cloneable and thread-safe: hand it to another Python thread that
/// waits on completion while your main thread polls progress.
#[pyclass(module = "rusty_desktop_icons", name = "AnimationHandle", skip_from_py_object)]
#[derive(Clone)]
pub struct PyAnimationHandle {
    pub(crate) inner: CoreHandle,
}

impl PyAnimationHandle {
    pub(crate) fn from_core(inner: CoreHandle) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyAnimationHandle {
    /// `True` while the animation is still ticking.
    fn is_running(&self) -> bool {
        self.inner.is_running()
    }

    /// Global progress `∈ [0, 1]` (mean per-icon `t`).
    fn progress(&self) -> f32 {
        self.inner.progress()
    }

    /// Copy of the current per-icon state.
    fn snapshot(&self) -> Vec<PyIconAnimationState> {
        self.inner
            .snapshot()
            .into_iter()
            .map(PyIconAnimationState::from_inner)
            .collect()
    }

    /// Ids listed in the spec but not present on the desktop at start.
    fn missing_icons(&self) -> Vec<String> {
        self.inner
            .missing_icons()
            .into_iter()
            .map(|id| id.into_string())
            .collect()
    }

    /// Read the finish reason if the animation has already ended.
    fn finish_reason(&self) -> Option<PyFinishReason> {
        self.inner.finish_reason().map(PyFinishReason::from_core)
    }

    /// Outcome of the Shell-side final commit, once the animation has
    /// finished. `None` while it is still running, or if finalization
    /// itself failed (in which case `finish_reason()` carries the error).
    fn final_commit(&self) -> Option<PyFinalCommitOutcome> {
        self.inner
            .final_commit()
            .map(PyFinalCommitOutcome::from_core)
    }

    // ---- control -------------------------------------------------------

    /// Ask the engine to stop the animation.
    ///
    /// `mode` defaults to [`StopMode.LeaveInPlace`]. This is the only way
    /// to alter a running animation — to change targets or the icon set,
    /// stop, read `snapshot()`, and call `animate()` again.
    #[pyo3(signature = (mode=None))]
    fn stop(&self, mode: Option<PyStopMode>) {
        let mode = mode.unwrap_or(PyStopMode::LeaveInPlace).to_core();
        self.inner.stop(mode);
    }

    // ---- waiting -------------------------------------------------------

    /// Block until the animation finishes. Releases the GIL while waiting.
    fn wait(&self, py: Python<'_>) -> PyFinishReason {
        let reason = py.detach(|| self.inner.wait());
        PyFinishReason::from_core(reason)
    }

    /// Block for up to `timeout_seconds`. Returns `None` on timeout.
    #[pyo3(signature = (timeout_seconds))]
    fn wait_timeout(
        &self,
        py: Python<'_>,
        timeout_seconds: f64,
    ) -> PyResult<Option<PyFinishReason>> {
        if !timeout_seconds.is_finite() || timeout_seconds < 0.0 {
            return Err(PyValueError::new_err(format!(
                "timeout_seconds must be a non-negative finite number, got {timeout_seconds}"
            )));
        }
        let d = StdDuration::try_from_secs_f64(timeout_seconds).map_err(|e| {
            PyValueError::new_err(format!(
                "timeout_seconds could not be represented: {e}"
            ))
        })?;
        Ok(py
            .detach(|| self.inner.wait_timeout(d))
            .map(PyFinishReason::from_core))
    }

    // ---- observers -----------------------------------------------------

    #[pyo3(signature = (cb: "typing.Callable[[StartContext], None]"))]
    fn on_start(&self, cb: Py<PyAny>) {
        let cb = Arc::new(cb);
        self.inner.on_start({
            let cb = cb.clone();
            move |ctx| {
                Python::attach(|py| {
                    let arg = Py::new(py, PyStartContext::from_inner(ctx));
                    match arg {
                        Ok(arg) => {
                            if let Err(e) = cb.call1(py, (arg,)) {
                                e.write_unraisable(py, Some(&cb.bind(py)));
                            }
                        }
                        Err(e) => e.write_unraisable(py, Some(&cb.bind(py))),
                    }
                });
            }
        });
    }

    #[pyo3(signature = (cb: "typing.Callable[[TickContext], None]"))]
    fn on_tick(&self, cb: Py<PyAny>) {
        let cb = Arc::new(cb);
        self.inner.on_tick({
            let cb = cb.clone();
            move |ctx| {
                Python::attach(|py| {
                    let arg = Py::new(py, PyTickContext::from_inner(ctx));
                    match arg {
                        Ok(arg) => {
                            if let Err(e) = cb.call1(py, (arg,)) {
                                e.write_unraisable(py, Some(&cb.bind(py)));
                            }
                        }
                        Err(e) => e.write_unraisable(py, Some(&cb.bind(py))),
                    }
                });
            }
        });
    }

    #[pyo3(signature = (cb: "typing.Callable[[str], None]"))]
    fn on_icon_complete(&self, cb: Py<PyAny>) {
        let cb = Arc::new(cb);
        self.inner.on_icon_complete({
            let cb = cb.clone();
            move |id| {
                Python::attach(|py| {
                    if let Err(e) = cb.call1(py, (id.as_str(),)) {
                        e.write_unraisable(py, Some(&cb.bind(py)));
                    }
                });
            }
        });
    }

    #[pyo3(signature = (cb: "typing.Callable[[FinishReason], None]"))]
    fn on_finish(&self, cb: Py<PyAny>) {
        let cb = Arc::new(cb);
        self.inner.on_finish({
            let cb = cb.clone();
            move |reason| {
                Python::attach(|py| {
                    let arg = Py::new(py, PyFinishReason::from_core(reason.clone()));
                    match arg {
                        Ok(arg) => {
                            if let Err(e) = cb.call1(py, (arg,)) {
                                e.write_unraisable(py, Some(&cb.bind(py)));
                            }
                        }
                        Err(e) => e.write_unraisable(py, Some(&cb.bind(py))),
                    }
                });
            }
        });
    }

    fn __repr__(&self) -> String {
        format!(
            "AnimationHandle(running={}, progress={:.3})",
            self.inner.is_running(),
            self.inner.progress()
        )
    }
}

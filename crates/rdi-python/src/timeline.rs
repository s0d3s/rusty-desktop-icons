use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::errors::map_desktop_error;
use crate::geometry::{PyFinalCommitOutcome, PyFinishReason, PyIconAnimationState};

#[pyclass(eq, eq_int, from_py_object, module = "rusty_desktop_icons", name = "TimelineCloseMode")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PyTimelineCloseMode {
    RestoreOrigins,
    LeaveInPlace,
    TeleportToTarget,
}

impl PyTimelineCloseMode {
    fn to_core(self) -> rdi_core::TimelineCloseMode {
        match self {
            Self::RestoreOrigins => rdi_core::TimelineCloseMode::RestoreOrigins,
            Self::LeaveInPlace => rdi_core::TimelineCloseMode::LeaveInPlace,
            Self::TeleportToTarget => rdi_core::TimelineCloseMode::TeleportToTarget,
        }
    }
}

#[pyclass(eq, eq_int, from_py_object, module = "rusty_desktop_icons", name = "TimelineState")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PyTimelineState {
    Paused,
    Playing,
    Closed,
}

#[pyclass(eq, eq_int, from_py_object, module = "rusty_desktop_icons", name = "PlaybackOutcome")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PyPlaybackOutcome {
    Reached,
    Interrupted,
    Closed,
}

impl PyPlaybackOutcome {
    fn from_core(outcome: rdi_core::PlaybackOutcome) -> Self {
        match outcome {
            rdi_core::PlaybackOutcome::Reached => Self::Reached,
            rdi_core::PlaybackOutcome::Interrupted => Self::Interrupted,
            rdi_core::PlaybackOutcome::Closed => Self::Closed,
        }
    }
}

#[pyclass(module = "rusty_desktop_icons", name = "PlaybackHandle")]
pub struct PyPlaybackHandle {
    inner: rdi_core::PlaybackHandle,
}

#[pymethods]
impl PyPlaybackHandle {
    fn wait(&self, py: Python<'_>) -> PyResult<PyPlaybackOutcome> {
        py.detach(|| self.inner.wait()).map(PyPlaybackOutcome::from_core).map_err(map_desktop_error)
    }

    fn wait_timeout(&self, py: Python<'_>, timeout_seconds: f64) -> PyResult<Option<PyPlaybackOutcome>> {
        let timeout = std::time::Duration::try_from_secs_f64(timeout_seconds)
            .map_err(|_| PyValueError::new_err("timeout_seconds must be non-negative, finite and representable"))?;
        py.detach(|| self.inner.wait_timeout(timeout))
            .map(|outcome| outcome.map(PyPlaybackOutcome::from_core)).map_err(map_desktop_error)
    }
}

/// Persistent, seekable playback opened by ``PreparedAnimation.open_timeline``.
///
/// Reaching an endpoint pauses without closing the overlay. Use as a context
/// manager to restore original icon positions on exit, including exceptions.
#[pyclass(module = "rusty_desktop_icons", name = "TimelineSession")]
pub struct PyTimelineSession {
    pub(crate) inner: rdi_core::TimelineSession,
    pub(crate) _owner: Py<crate::controller::PyDesktopController>,
}

#[pymethods]
impl PyTimelineSession {
    /// Capture the last submitted overlay frame, without changing playback.
    fn capture(&self, py: Python<'_>) -> PyResult<crate::scene::PyCapturedFrame> {
        py.detach(|| self.inner.capture()).map(|inner| crate::scene::PyCapturedFrame { inner }).map_err(map_desktop_error)
    }

    /// Pause, seek and capture as one worker operation. Position is normalized.
    fn seek_and_capture(&self, py: Python<'_>, position: f64) -> PyResult<crate::scene::PyCapturedFrame> {
        py.detach(|| self.inner.seek_and_capture(position)).map(|inner| crate::scene::PyCapturedFrame { inner }).map_err(map_desktop_error)
    }

    /// Play toward a normalized position in [0, 1] at a positive finite speed.
    #[pyo3(signature = (position, speed=1.0))]
    fn play_to(&self, py: Python<'_>, position: f64, speed: f64) -> PyResult<PyPlaybackHandle> {
        py.detach(|| self.inner.play_to(position, speed))
            .map(|inner| PyPlaybackHandle { inner }).map_err(map_desktop_error)
    }

    /// Pause and show the requested normalized position in [0, 1].
    fn seek(&self, py: Python<'_>, position: f64) -> PyResult<()> {
        py.detach(|| self.inner.seek(position)).map_err(map_desktop_error)
    }

    fn pause(&self, py: Python<'_>) -> PyResult<()> {
        py.detach(|| self.inner.pause()).map_err(map_desktop_error)
    }

    fn set_speed(&self, py: Python<'_>, speed: f64) -> PyResult<()> {
        py.detach(|| self.inner.set_speed(speed)).map_err(map_desktop_error)
    }

    fn position(&self) -> f64 { self.inner.position() }
    fn real_icons_visible(&self) -> bool { self.inner.real_icons_visible() }

    fn set_real_icons_visible(&self, py: Python<'_>, visible: bool) -> PyResult<()> {
        py.detach(|| self.inner.set_real_icons_visible(visible)).map_err(map_desktop_error)
    }

    fn speed(&self) -> f64 { self.inner.speed() }

    fn state(&self) -> PyTimelineState {
        match self.inner.state() {
            rdi_core::TimelineState::Paused => PyTimelineState::Paused,
            rdi_core::TimelineState::Playing => PyTimelineState::Playing,
            rdi_core::TimelineState::Closed => PyTimelineState::Closed,
        }
    }

    fn snapshot(&self) -> Vec<PyIconAnimationState> {
        self.inner.snapshot().into_iter().map(PyIconAnimationState::from_inner).collect()
    }

    fn missing_icons(&self) -> Vec<String> {
        self.inner.missing_icons().into_iter().map(|id| id.into_string()).collect()
    }

    fn finish_reason(&self) -> Option<PyFinishReason> {
        self.inner.finish_reason().map(PyFinishReason::from_core)
    }

    fn final_commit(&self) -> Option<PyFinalCommitOutcome> {
        self.inner.final_commit().map(PyFinalCommitOutcome::from_core)
    }

    /// Close the overlay, restoring original positions unless another mode is given.
    #[pyo3(signature = (mode=None))]
    fn close(&self, py: Python<'_>, mode: Option<PyTimelineCloseMode>) -> PyResult<PyFinishReason> {
        let mode = mode.unwrap_or(PyTimelineCloseMode::RestoreOrigins).to_core();
        py.detach(|| self.inner.close(mode)).map(PyFinishReason::from_core).map_err(map_desktop_error)
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> { slf }

    #[pyo3(signature = (_ty: "object", _value: "object", _traceback: "object"))]
    fn __exit__(&self, py: Python<'_>, _ty: &Bound<'_, PyAny>, _value: &Bound<'_, PyAny>, _traceback: &Bound<'_, PyAny>) -> PyResult<()> {
        self.close(py, None).map(|_| ())
    }
}
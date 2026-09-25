use pyo3::prelude::*;
use pyo3::types::PyBytes;
use crate::errors::map_desktop_error;

/// Off-screen dimensions in physical pixels; icon_size is the baseline DIP slot size.
#[pyclass(frozen, from_py_object, module = "rusty_desktop_icons", name = "Canvas")]
#[derive(Clone)]
pub struct PyCanvas {
    pub(crate) inner: rdi_core::Canvas,
}

#[pymethods]
impl PyCanvas {
    #[new]
    #[pyo3(signature = (width, height, *, dpi_scale=1.0, icon_size=48))]
    fn new(width: u32, height: u32, dpi_scale: f32, icon_size: u32) -> PyResult<Self> {
        let inner = rdi_core::Canvas { width, height, dpi_scale, icon_size };
        inner.validate().map_err(map_desktop_error)?;
        Ok(Self { inner })
    }
    #[getter]
    fn width(&self) -> u32 { self.inner.width }
    #[getter]
    fn height(&self) -> u32 { self.inner.height }
    #[getter]
    fn dpi_scale(&self) -> f32 { self.inner.dpi_scale }
    #[getter]
    fn icon_size(&self) -> u32 { self.inner.icon_size }
}

/// Owned, tightly packed premultiplied BGRA pixels. Convert alpha before saving PNG.
#[pyclass(frozen, module = "rusty_desktop_icons", name = "CapturedFrame")]
pub struct PyCapturedFrame {
    pub(crate) inner: rdi_core::CapturedFrame,
}

#[pymethods]
impl PyCapturedFrame {
    #[getter]
    fn width(&self) -> u32 { self.inner.width }
    #[getter]
    fn height(&self) -> u32 { self.inner.height }
    #[getter]
    fn stride(&self) -> u32 { self.inner.width * 4 }
    #[getter]
    fn seconds(&self) -> f64 { self.inner.seconds }
    #[getter]
    fn pixel_format(&self) -> &'static str { "BGRA8_PREMULTIPLIED" }
    #[getter]
    fn pixels<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> { PyBytes::new(py, &self.inner.pixels) }
}

/// Prepared off-screen scene. No window, Shell position writes or visibility changes.
/// Use as a context manager; render_at samples exact seconds, independent of wall time.
#[pyclass(module = "rusty_desktop_icons", name = "RenderSession")]
pub struct PyRenderSession {
    pub(crate) inner: rdi_core::RenderSession,
    pub(crate) _owner: Py<crate::controller::PyDesktopController>,
}

#[pymethods]
impl PyRenderSession {
    #[getter]
    fn duration(&self) -> f64 { self.inner.duration }

    fn render_at(&self, py: Python<'_>, seconds: f64) -> PyResult<PyCapturedFrame> {
        py.detach(|| self.inner.render_at(seconds)).map(|inner| PyCapturedFrame { inner }).map_err(map_desktop_error)
    }

    fn close(&self, py: Python<'_>) -> PyResult<()> {
        py.detach(|| self.inner.close()).map_err(map_desktop_error)
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> { slf }

    #[pyo3(signature = (_ty: "object", _value: "object", _traceback: "object"))]
    fn __exit__(&self, py: Python<'_>, _ty: &Bound<'_, PyAny>, _value: &Bound<'_, PyAny>, _traceback: &Bound<'_, PyAny>) -> PyResult<()> {
        self.close(py)
    }
}
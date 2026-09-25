//! Python shader values and one-shot prepared animations.

use crate::curve::PyCurve;
use crate::errors::map_desktop_error;
use crate::handle::PyAnimationHandle;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyType;
use std::sync::Mutex;

#[pyclass(
    frozen,
    from_py_object,
    module = "rusty_desktop_icons",
    name = "ShaderSource"
)]
#[derive(Clone)]
pub struct PyShaderSource {
    #[pyo3(get)]
    execution: Option<crate::procedural::PyProceduralSource>,
    #[pyo3(get)]
    pixel: Option<String>,
    #[pyo3(get)]
    vertex: Option<String>,
    pipeline: rdi_core::ShaderPipeline,
}

#[pymethods]
impl PyShaderSource {
    #[new]
    #[pyo3(signature = (pixel=None, *, vertex=None, pipeline: "ShaderPipeline | str"="sprite", execution=None))]
    fn new(pixel: Option<String>, vertex: Option<String>, pipeline: &str, execution: Option<crate::procedural::PyProceduralSource>) -> PyResult<Self> {
        let pipeline = match pipeline {
            "sprite" => rdi_core::ShaderPipeline::Sprite,
            "particles" => rdi_core::ShaderPipeline::Particles,
            "procedural" => rdi_core::ShaderPipeline::Procedural,
            _ => {
                return Err(map_desktop_error(rdi_core::DesktopError::InvalidEffect(
                    format!("unknown shader pipeline: {pipeline}; expected sprite, particles or procedural"),
                )));
            }
        };
        Ok(Self {
            execution,
            pixel,
            vertex,
            pipeline,
        })
    }

    #[getter]
    fn pipeline(&self) -> crate::enums::ShaderPipeline {
        let value = match self.pipeline {
            rdi_core::ShaderPipeline::Sprite => "sprite",
            rdi_core::ShaderPipeline::Particles => "particles",
            rdi_core::ShaderPipeline::Procedural => "procedural",
        };
        crate::enums::ShaderPipeline(value)
    }

    #[classmethod]
    #[pyo3(signature = (name: "BuiltinShader | str"))]
    fn builtin(_cls: &Bound<'_, PyType>, name: &str) -> PyResult<Self> {
        #[cfg(windows)]
        {
            let builtin = name
                .parse::<rdi_platform_windows::shader::BuiltinShader>()
                .map_err(map_desktop_error)?;
            let source = builtin.source();
            Ok(Self {
                execution: source.execution.as_deref().map(crate::procedural::PyProceduralSource::from_native),
                pixel: source.pixel.map(str::to_owned),
                vertex: source.vertex.map(str::to_owned),
                pipeline: source.pipeline,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = name;
            Err(map_desktop_error(
                rdi_core::DesktopError::UnsupportedPlatform,
            ))
        }
    }
}

#[pyclass(
    frozen,
    from_py_object,
    module = "rusty_desktop_icons",
    name = "Shader"
)]
#[derive(Clone)]
pub struct PyShader {
    pub(crate) inner: rdi_core::ShaderProgram,
}

#[pymethods]
impl PyShader {
    #[classmethod]
    #[pyo3(signature = (source: "str | ShaderSource"))]
    fn compile(
        _cls: &Bound<'_, PyType>,
        py: Python<'_>,
        source: &Bound<'_, PyAny>,
    ) -> PyResult<Self> {
        let source = if let Ok(pixel) = source.extract::<String>() {
            PyShaderSource::new(Some(pixel), None, "sprite", None)?
        } else {
            source.extract::<PyShaderSource>()?
        };
        #[cfg(windows)]
        {
            let source = rdi_platform_windows::shader::ShaderSource {
                execution: source.execution.as_ref().map(|execution| execution.to_native().map(std::sync::Arc::new)).transpose()?,
                pipeline: source.pipeline,
                pixel: source.pixel.as_deref(),
                vertex: source.vertex.as_deref(),
            };
            py.detach(|| rdi_platform_windows::shader::compile(source))
                .map(|inner| Self { inner })
                .map_err(map_desktop_error)
        }
        #[cfg(not(windows))]
        {
            let _ = (py, source);
            Err(map_desktop_error(
                rdi_core::DesktopError::UnsupportedPlatform,
            ))
        }
    }

    #[getter]
    fn bytecode_size(&self) -> usize {
        self.inner.pixel_bytecode.len() + self.inner.vertex_bytecode.len()
    }
}

#[pyclass(
    frozen,
    from_py_object,
    module = "rusty_desktop_icons",
    name = "Effect"
)]
#[derive(Clone)]
pub struct PyEffect {
    pub(crate) inner: rdi_core::Effect,
}

#[pymethods]
impl PyEffect {
    #[new]
    #[pyo3(signature = (shader, *, params=None, parameters=None, envelope=None, padding_px=16, seed=0.0))]
    fn new(
        mut shader: PyShader,
        params: Option<(f32, f32, f32, f32)>,
        parameters: Option<std::collections::BTreeMap<String, f32>>,
        envelope: Option<PyCurve>,
        padding_px: u32,
        seed: f32,
    ) -> PyResult<Self> {
        if let Some(parameters) = parameters {
            if params.is_some() { return Err(map_desktop_error(rdi_core::DesktopError::InvalidEffect("use params or parameters, not both".into()))); }
            shader.inner = shader.inner.with_parameters(&parameters).map_err(map_desktop_error)?;
        }
        let envelope = envelope.map(|curve| curve.inner).unwrap_or_else(|| {
            if shader.inner.pipeline == rdi_core::ShaderPipeline::Procedural {
                return rdi_core::Curve::keyframes(
                    vec![rdi_core::Keyframe::new(0.0, 1.0), rdi_core::Keyframe::new(1.0, 1.0)],
                    rdi_core::KeyframeInterp::Linear,
                ).expect("valid constant envelope");
            }
            rdi_core::Curve::keyframes(
                vec![
                    rdi_core::Keyframe::new(0.0, 0.0),
                    rdi_core::Keyframe::new(0.15, 1.0),
                    rdi_core::Keyframe::new(0.85, 1.0),
                    rdi_core::Keyframe::new(1.0, 0.0),
                ],
                rdi_core::KeyframeInterp::SmoothStep,
            )
            .expect("valid built-in envelope")
        });
        let params = params
            .map(|params| [params.0, params.1, params.2, params.3])
            .unwrap_or_else(|| shader.inner.default_params());
        let inner = rdi_core::Effect {
            shader: shader.inner,
            params,
            padding_px,
            envelope,
            seed,
        };
        inner.validate().map_err(map_desktop_error)?;
        Ok(Self { inner })
    }

    #[getter]
    fn padding_px(&self) -> u32 {
        self.inner.padding_px
    }

    #[getter]
    fn parameters(&self) -> std::collections::BTreeMap<String, f32> {
        let values = self.inner.parameter_values();
        self.inner.shader.execution.as_ref().map(|execution| execution.parameters.iter().zip(values)
            .map(|(parameter, value)| (parameter.name.clone(), value)).collect()).unwrap_or_default()
    }

    #[getter]
    fn params(&self) -> (f32, f32, f32, f32) {
        let params = self.inner.params;
        (params[0], params[1], params[2], params[3])
    }
}

#[pyclass(module = "rusty_desktop_icons", name = "PreparedAnimation")]
pub struct PyPreparedAnimation {
    pub(crate) inner: Mutex<Option<rdi_core::PreparedAnimation>>,
    pub(crate) _owner: Py<crate::controller::PyDesktopController>,
}

#[pymethods]
impl PyPreparedAnimation {
    fn open_timeline(&self, py: Python<'_>) -> PyResult<crate::timeline::PyTimelineSession> {
        let prepared = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("prepared animation already consumed"))?;
        let inner = py
            .detach(|| prepared.open_timeline())
            .map_err(map_desktop_error)?;
        Ok(crate::timeline::PyTimelineSession {
            inner,
            _owner: self._owner.clone_ref(py),
        })
    }

    /// Consume the prepared session. No compilation or artwork upload occurs here.
    fn start(&self, py: Python<'_>) -> PyResult<PyAnimationHandle> {
        let prepared = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("prepared animation already consumed"))?;
        py.detach(|| prepared.start())
            .map(PyAnimationHandle::from_core)
            .map_err(map_desktop_error)
    }

    /// Release an unused session; idempotent. Does not move desktop icons.
    fn cancel(&self, py: Python<'_>) {
        let prepared = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        if let Some(prepared) = prepared {
            py.detach(|| prepared.cancel());
        }
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    #[pyo3(signature = (_ty: "object", _value: "object", _traceback: "object"))]
    fn __exit__(
        &self,
        py: Python<'_>,
        _ty: &Bound<'_, PyAny>,
        _value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) {
        self.cancel(py);
    }
}

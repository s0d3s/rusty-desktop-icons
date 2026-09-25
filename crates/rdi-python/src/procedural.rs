use crate::errors::map_desktop_error;
use pyo3::prelude::*;

fn invalid(message: &str) -> PyErr {
    map_desktop_error(rdi_core::DesktopError::InvalidEffect(message.into()))
}

#[pyclass(
    frozen,
    from_py_object,
    module = "rusty_desktop_icons",
    name = "EffectParameter"
)]
#[derive(Clone)]
pub struct PyEffectParameter {
    #[pyo3(get)]
    name: String,
    #[pyo3(get)]
    kind: String,
    #[pyo3(get)]
    default: f32,
    #[pyo3(get)]
    min: f32,
    #[pyo3(get)]
    max: f32,
}

#[pymethods]
impl PyEffectParameter {
    #[new]
    #[pyo3(signature = (name, default, *, min, max, kind="float"))]
    fn new(name: String, default: f32, min: f32, max: f32, kind: &str) -> PyResult<Self> {
        let result = Self {
            name,
            default,
            min,
            max,
            kind: kind.into(),
        };
        let parameter = result.to_core()?;
        if !parameter.accepts(default) || !min.is_finite() || !max.is_finite() {
            return Err(invalid("invalid parameter bounds or default"));
        }
        Ok(result)
    }
}

impl PyEffectParameter {
    fn to_core(&self) -> PyResult<rdi_core::EffectParameter> {
        let kind = match self.kind.as_str() {
            "float" => rdi_core::ParameterKind::Float,
            "integer" => rdi_core::ParameterKind::Integer,
            "boolean" => rdi_core::ParameterKind::Boolean,
            _ => return Err(invalid("parameter kind must be float, integer or boolean")),
        };
        Ok(rdi_core::EffectParameter {
            name: self.name.clone(),
            kind,
            default: self.default,
            min: self.min,
            max: self.max,
        })
    }
}

#[pyclass(
    frozen,
    from_py_object,
    module = "rusty_desktop_icons",
    name = "RenderTarget"
)]
#[derive(Clone)]
pub struct PyRenderTarget {
    #[pyo3(get)]
    width: u32,
    #[pyo3(get)]
    height: u32,
}

#[pymethods]
impl PyRenderTarget {
    #[new]
    fn new(width: u32, height: u32) -> PyResult<Self> {
        if width == 0 || height == 0 || width > 8192 || height > 8192 {
            return Err(invalid("target axes must be in 1..8192"));
        }
        Ok(Self { width, height })
    }
}

#[pyclass(
    frozen,
    from_py_object,
    module = "rusty_desktop_icons",
    name = "RenderPass"
)]
#[derive(Clone)]
pub struct PyRenderPass {
    #[pyo3(get)]
    vertex: Option<String>,
    #[pyo3(get)]
    pixel: Option<String>,
    #[pyo3(get)]
    vertices: u32,
    #[pyo3(get)]
    count_parameter: Option<String>,
    #[pyo3(get)]
    multiplier: u32,
    #[pyo3(get)]
    topology: String,
    #[pyo3(get)]
    inputs: Vec<usize>,
    #[pyo3(get)]
    output: Option<usize>,
    #[pyo3(get)]
    blend: String,
}

#[pymethods]
impl PyRenderPass {
    #[new]
    #[pyo3(signature = (*, vertex=None, pixel=None, vertices=6, count_parameter=None, multiplier=0, topology="triangles", inputs=None, output=None, blend="over"))]
    fn new(
        vertex: Option<String>,
        pixel: Option<String>,
        vertices: u32,
        count_parameter: Option<String>,
        multiplier: u32,
        topology: &str,
        inputs: Option<Vec<usize>>,
        output: Option<usize>,
        blend: &str,
    ) -> PyResult<Self> {
        if !matches!(topology, "triangles" | "triangle-strip" | "lines")
            || !matches!(blend, "over" | "add" | "replace")
        {
            return Err(invalid("unknown topology or blend mode"));
        }
        Ok(Self {
            vertex,
            pixel,
            vertices,
            count_parameter,
            multiplier,
            topology: topology.into(),
            inputs: inputs.unwrap_or_default(),
            output,
            blend: blend.into(),
        })
    }
}

#[pyclass(
    frozen,
    from_py_object,
    module = "rusty_desktop_icons",
    name = "ProceduralSource"
)]
#[derive(Clone)]
pub struct PyProceduralSource {
    #[pyo3(get)]
    parameters: Vec<PyEffectParameter>,
    #[pyo3(get)]
    targets: Vec<PyRenderTarget>,
    #[pyo3(get)]
    passes: Vec<PyRenderPass>,
    #[pyo3(get)]
    body_artwork: bool,
    #[pyo3(get)]
    label_artwork: bool,
}

#[pymethods]
impl PyProceduralSource {
    #[new]
    #[pyo3(signature = (passes, *, parameters=None, targets=None, body_artwork=false, label_artwork=false))]
    fn new(
        passes: Vec<PyRenderPass>,
        parameters: Option<Vec<PyEffectParameter>>,
        targets: Option<Vec<PyRenderTarget>>,
        body_artwork: bool,
        label_artwork: bool,
    ) -> Self {
        Self {
            passes,
            parameters: parameters.unwrap_or_default(),
            targets: targets.unwrap_or_default(),
            body_artwork,
            label_artwork,
        }
    }
}

#[cfg(windows)]
impl PyProceduralSource {
    pub(crate) fn to_native(&self) -> PyResult<rdi_platform_windows::shader::ExecutionSource> {
        use rdi_core::{DrawSpec, DrawTopology, PassBlend};
        let parameters = self
            .parameters
            .iter()
            .map(PyEffectParameter::to_core)
            .collect::<PyResult<Vec<_>>>()?;
        let passes = self
            .passes
            .iter()
            .map(|pass| {
                let parameter = pass
                    .count_parameter
                    .as_ref()
                    .map(|name| {
                        parameters
                            .iter()
                            .position(|parameter| parameter.name == *name)
                            .ok_or_else(|| invalid("unknown count parameter"))
                    })
                    .transpose()?;
                Ok(rdi_platform_windows::shader::PassSource {
                    vertex: pass.vertex.clone(),
                    pixel: pass.pixel.clone(),
                    inputs: pass.inputs.clone(),
                    output: pass.output,
                    draw: DrawSpec {
                        vertices: pass.vertices,
                        parameter,
                        multiplier: pass.multiplier,
                        topology: match pass.topology.as_str() {
                            "triangles" => DrawTopology::Triangles,
                            "triangle-strip" => DrawTopology::TriangleStrip,
                            _ => DrawTopology::Lines,
                        },
                    },
                    blend: match pass.blend.as_str() {
                        "over" => PassBlend::Over,
                        "add" => PassBlend::Add,
                        _ => PassBlend::Replace,
                    },
                })
            })
            .collect::<PyResult<Vec<_>>>()?;
        Ok(rdi_platform_windows::shader::ExecutionSource {
            parameters,
            passes,
            targets: self
                .targets
                .iter()
                .map(|target| rdi_core::EffectTarget {
                    width: target.width,
                    height: target.height,
                })
                .collect(),
            body_artwork: self.body_artwork,
            label_artwork: self.label_artwork,
        })
    }

    pub(crate) fn from_native(source: &rdi_platform_windows::shader::ExecutionSource) -> Self {
        Self {
            parameters: source
                .parameters
                .iter()
                .map(|parameter| PyEffectParameter {
                    name: parameter.name.clone(),
                    kind: match parameter.kind {
                        rdi_core::ParameterKind::Float => "float",
                        rdi_core::ParameterKind::Integer => "integer",
                        rdi_core::ParameterKind::Boolean => "boolean",
                    }
                    .into(),
                    default: parameter.default,
                    min: parameter.min,
                    max: parameter.max,
                })
                .collect(),
            targets: source
                .targets
                .iter()
                .map(|target| PyRenderTarget {
                    width: target.width,
                    height: target.height,
                })
                .collect(),
            passes: source
                .passes
                .iter()
                .map(|pass| PyRenderPass {
                    vertex: pass.vertex.clone(),
                    pixel: pass.pixel.clone(),
                    vertices: pass.draw.vertices,
                    count_parameter: pass
                        .draw
                        .parameter
                        .map(|index| source.parameters[index].name.clone()),
                    multiplier: pass.draw.multiplier,
                    topology: match pass.draw.topology {
                        rdi_core::DrawTopology::Triangles => "triangles",
                        rdi_core::DrawTopology::TriangleStrip => "triangle-strip",
                        rdi_core::DrawTopology::Lines => "lines",
                    }
                    .into(),
                    inputs: pass.inputs.clone(),
                    output: pass.output,
                    blend: match pass.blend {
                        rdi_core::PassBlend::Over => "over",
                        rdi_core::PassBlend::Add => "add",
                        rdi_core::PassBlend::Replace => "replace",
                    }
                    .into(),
                })
                .collect(),
            body_artwork: source.body_artwork,
            label_artwork: source.label_artwork,
        }
    }
}

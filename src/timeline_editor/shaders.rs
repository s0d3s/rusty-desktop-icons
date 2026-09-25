use rdi_core::{
    DrawSpec, DrawTopology, Effect, EffectParameter, EffectTarget, ParameterKind, PassBlend,
    ShaderPipeline, ShaderProgram,
};
use rdi_platform_windows::shader::{
    self, BuiltinShader, ExecutionSource, PassSource, ShaderSource,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub params: [f32; 4],
    pub padding_px: u32,
    pub seed: f32,
    pub parameters: Vec<Parameter>,
    pub targets: Vec<[u32; 2]>,
    pub passes: Vec<Pass>,
    pub body_artwork: bool,
    pub label_artwork: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    name: String,
    kind: String,
    default: f32,
    min: f32,
    max: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pass {
    vertex: Option<String>,
    pixel: Option<String>,
    vertices: u32,
    parameter: Option<usize>,
    multiplier: u32,
    topology: String,
    inputs: Vec<usize>,
    output: Option<usize>,
    blend: String,
}

impl Settings {
    pub fn execution(&self) -> Result<ExecutionSource, String> {
        let parameters = self
            .parameters
            .iter()
            .map(|parameter| {
                Ok(EffectParameter {
                    name: parameter.name.clone(),
                    default: parameter.default,
                    min: parameter.min,
                    max: parameter.max,
                    kind: match parameter.kind.as_str() {
                        "float" => ParameterKind::Float,
                        "integer" => ParameterKind::Integer,
                        "boolean" => ParameterKind::Boolean,
                        _ => return Err("Parameter kind: float, integer or boolean".into()),
                    },
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let passes = self
            .passes
            .iter()
            .map(|pass| {
                Ok(PassSource {
                    vertex: pass.vertex.clone(),
                    pixel: pass.pixel.clone(),
                    inputs: pass.inputs.clone(),
                    output: pass.output,
                    draw: DrawSpec {
                        vertices: pass.vertices,
                        parameter: pass.parameter,
                        multiplier: pass.multiplier,
                        topology: match pass.topology.as_str() {
                            "triangles" => DrawTopology::Triangles,
                            "triangle-strip" => DrawTopology::TriangleStrip,
                            "lines" => DrawTopology::Lines,
                            _ => return Err("Topology: triangles, triangle-strip or lines".into()),
                        },
                    },
                    blend: match pass.blend.as_str() {
                        "over" => PassBlend::Over,
                        "add" => PassBlend::Add,
                        "replace" => PassBlend::Replace,
                        _ => return Err("Blend: over, add or replace".into()),
                    },
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(ExecutionSource {
            parameters,
            passes,
            targets: self
                .targets
                .iter()
                .map(|size| EffectTarget {
                    width: size[0],
                    height: size[1],
                })
                .collect(),
            body_artwork: self.body_artwork,
            label_artwork: self.label_artwork,
        })
    }

    fn from_execution(execution: ExecutionSource, params: [f32; 4]) -> Self {
        Self {
            params,
            padding_px: 24,
            seed: 0.0,
            parameters: execution
                .parameters
                .into_iter()
                .map(|parameter| Parameter {
                    name: parameter.name,
                    default: parameter.default,
                    min: parameter.min,
                    max: parameter.max,
                    kind: match parameter.kind {
                        ParameterKind::Float => "float",
                        ParameterKind::Integer => "integer",
                        ParameterKind::Boolean => "boolean",
                    }
                    .into(),
                })
                .collect(),
            targets: execution
                .targets
                .into_iter()
                .map(|target| [target.width, target.height])
                .collect(),
            passes: execution
                .passes
                .into_iter()
                .map(|pass| Pass {
                    vertex: pass.vertex,
                    pixel: pass.pixel,
                    vertices: pass.draw.vertices,
                    parameter: pass.draw.parameter,
                    multiplier: pass.draw.multiplier,
                    inputs: pass.inputs,
                    output: pass.output,
                    topology: match pass.draw.topology {
                        DrawTopology::Triangles => "triangles",
                        DrawTopology::TriangleStrip => "triangle-strip",
                        DrawTopology::Lines => "lines",
                    }
                    .into(),
                    blend: match pass.blend {
                        PassBlend::Over => "over",
                        PassBlend::Add => "add",
                        PassBlend::Replace => "replace",
                    }
                    .into(),
                })
                .collect(),
            body_artwork: execution.body_artwork,
            label_artwork: execution.label_artwork,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ShaderDraft {
    pub base: BuiltinShader,
    pub pipeline: ShaderPipeline,
    pub vertex: String,
    pub pixel: String,
    pub default_vertex: bool,
    pub default_pixel: bool,
    pub settings: String,
}

impl ShaderDraft {
    pub fn from_builtin(base: BuiltinShader) -> Self {
        let source = base.source();
        let params = match source.pipeline {
            ShaderPipeline::Sprite => [8.0, 3.0, 6.0, 30.0],
            ShaderPipeline::Particles => [3.0, 1.5, 2.0, 0.65],
            ShaderPipeline::Procedural => {
                let mut params = [0.0; 4];
                if let Some(execution) = &source.execution {
                    for (target, parameter) in params.iter_mut().zip(&execution.parameters) {
                        *target = parameter.default;
                    }
                }
                params
            }
        };
        let execution = source
            .execution
            .as_deref()
            .cloned()
            .unwrap_or_else(|| ExecutionSource {
                passes: vec![PassSource::default()],
                ..Default::default()
            });
        Self {
            base, pipeline: source.pipeline, default_vertex: source.vertex.is_none(), default_pixel: source.pixel.is_none(),
            vertex: source.vertex.unwrap_or(rdi_platform_windows::shader_library::DEFAULT_VERTEX).into(),
            pixel: source.pixel.unwrap_or(rdi_platform_windows::shader_library::DEFAULT_PIXEL).into(),
            settings: serde_json::to_string_pretty(&Settings::from_execution(execution, params)).expect("finite built-in settings"),
        }
    }

    pub fn compile(&self) -> Result<(ShaderProgram, Settings), String> {
        if self.settings.len() + self.vertex.len() + self.pixel.len() > 2 * 1024 * 1024 {
            return Err("Shader draft exceeds 2 MiB".into());
        }
        let settings: Settings =
            serde_json::from_str(&self.settings).map_err(|error| error.to_string())?;
        let execution = if self.pipeline == ShaderPipeline::Procedural {
            Some(Arc::new(settings.execution()?))
        } else {
            None
        };
        let program = shader::compile(ShaderSource {
            pipeline: self.pipeline,
            execution,
            vertex: (!self.default_vertex).then_some(self.vertex.as_str()),
            pixel: (!self.default_pixel).then_some(self.pixel.as_str()),
        })
        .map_err(|error| error.to_string())?;
        Effect {
            shader: program.clone(),
            params: settings.params,
            padding_px: settings.padding_px,
            seed: settings.seed,
            envelope: self.base.default_preset().envelope,
        }
        .validate()
        .map_err(|error| error.to_string())?;
        Ok((program, settings))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editor_copies_every_shader_and_recipe() {
        for &builtin in BuiltinShader::ALL {
            let draft = ShaderDraft::from_builtin(builtin);
            let (program, _) = draft.compile().unwrap();
            let original = shader::compile(builtin).unwrap();
            assert_eq!(program, original);
        }
    }
    #[test]
    fn invalid_shader_and_settings_do_not_compile() {
        let mut draft = ShaderDraft::from_builtin(BuiltinShader::Glitch);
        draft.pixel = "invalid HLSL".into();
        assert!(draft.compile().is_err());
        draft.default_pixel = true;
        assert!(draft.compile().is_ok());
        draft.settings = "{}".into();
        assert!(draft.compile().is_err());
    }
}

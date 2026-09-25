//! Runtime HLSL compilation for the Windows sprite renderer.
//!
//! WARNING: This proof of concept (PoC) is not shader-injection-proof.
//! Only use shaders from sources you trust. Use untrusted shaders at your own risk.

pub use crate::shader_library::{BuiltinShader, GLITCH, IDENTITY, PARTICLE_VORTEX};
use crate::shader_library::{
    DEFAULT_PIXEL, DEFAULT_VERTEX, HEADER, PIXEL_DECLARATION, VERTEX_DECLARATION, VERTEX_FOOTER,
    pipeline_template,
};
use rdi_core::{DesktopError, ShaderPipeline, ShaderProgram};
use windows::Win32::Graphics::Direct3D::Fxc::{
    D3DCOMPILE_ENABLE_STRICTNESS, D3DCOMPILE_OPTIMIZATION_LEVEL3, D3DCompile,
};
use windows::Win32::Graphics::Direct3D::ID3DBlob;
use windows::core::{PCSTR, s};

#[derive(Clone, Debug, Default)]
pub struct PassSource {
    pub vertex: Option<String>,
    pub pixel: Option<String>,
    pub draw: rdi_core::DrawSpec,
    pub inputs: Vec<usize>,
    pub output: Option<usize>,
    pub blend: rdi_core::PassBlend,
}

#[derive(Clone, Debug, Default)]
pub struct ExecutionSource {
    pub parameters: Vec<rdi_core::EffectParameter>,
    pub targets: Vec<rdi_core::EffectTarget>,
    pub passes: Vec<PassSource>,
    pub body_artwork: bool,
    pub label_artwork: bool,
}

#[derive(Clone, Debug)]
pub struct ShaderSource<'a> {
    pub execution: Option<std::sync::Arc<ExecutionSource>>,
    pub pipeline: ShaderPipeline,
    pub pixel: Option<&'a str>,
    pub vertex: Option<&'a str>,
}

impl<'a> From<&'a str> for ShaderSource<'a> {
    fn from(pixel: &'a str) -> Self {
        Self {
            execution: None,
            pipeline: ShaderPipeline::Sprite,
            pixel: Some(pixel),
            vertex: None,
        }
    }
}

impl<'a> From<&'a String> for ShaderSource<'a> {
    fn from(pixel: &'a String) -> Self {
        pixel.as_str().into()
    }
}

/// Compile a complete trusted shader program from a descriptor or pixel-only HLSL.
/// No filesystem includes or GPU resources are accessed by this operation.
pub fn compile<'a>(source: impl Into<ShaderSource<'a>>) -> Result<ShaderProgram, DesktopError> {
    let source = source.into();
    if source.execution.is_some() && source.pipeline != ShaderPipeline::Procedural {
        return Err(DesktopError::InvalidEffect("execution requires procedural pipeline".into()));
    }
    let mut program = compile_stages(&source)?;
    if source.pipeline == ShaderPipeline::Procedural {
        let recipe = source.execution.as_deref().cloned().unwrap_or_else(|| ExecutionSource {
            passes: vec![PassSource::default()], ..Default::default()
        });
        if recipe.passes.len() > 8 || recipe.parameters.len() > 16 || recipe.targets.len() > 4 {
            return Err(DesktopError::InvalidEffect("procedural descriptor exceeds limits".into()));
        }
        let source_bytes: usize = recipe.passes.iter().map(|pass| pass.vertex.as_ref().map_or(0, String::len) + pass.pixel.as_ref().map_or(0, String::len)).sum();
        if source_bytes > 1024 * 1024 { return Err(DesktopError::InvalidEffect("pass sources exceed 1 MiB".into())); }
        let mut passes = Vec::new();
        for pass in recipe.passes {
            let stages = if pass.vertex.is_none() && pass.pixel.is_none() { program.clone() } else {
                compile_stages(&ShaderSource { execution: None, pipeline: ShaderPipeline::Procedural,
                    vertex: pass.vertex.as_deref().or(source.vertex), pixel: pass.pixel.as_deref().or(source.pixel) })?
            };
            passes.push(rdi_core::EffectPass { vertex_bytecode: stages.vertex_bytecode, pixel_bytecode: stages.pixel_bytecode,
                draw: pass.draw, inputs: pass.inputs, output: pass.output, blend: pass.blend });
        }
        let execution = rdi_core::EffectExecution { parameters: recipe.parameters, targets: recipe.targets, passes,
            body_artwork: recipe.body_artwork, label_artwork: recipe.label_artwork };
        execution.validate(&execution.defaults())?;
        program.execution = Some(std::sync::Arc::new(execution));
    }
    Ok(program)
}

fn compile_stages(source: &ShaderSource<'_>) -> Result<ShaderProgram, DesktopError> {
    if source
        .pixel
        .map_or(0, str::len)
        .saturating_add(source.vertex.map_or(0, str::len))
        > 1024 * 1024
    {
        return Err(DesktopError::InvalidEffect(
            "shader source exceeds 1 MiB".into(),
        ));
    }
    let (header, footer, pixel_entry) = pipeline_template(source.pipeline);
    let vertex_source = source.vertex.unwrap_or(DEFAULT_VERTEX);
    let pixel_source = source.pixel.unwrap_or(DEFAULT_PIXEL);
    let vertex = format!(
        "{HEADER}{header}\n{VERTEX_DECLARATION}\n#line 1 \"vertex.hlsl\"\n{vertex_source}\n#line 1 \"rdi_wrapper.hlsl\"\n{VERTEX_FOOTER}"
    );
    let pixel = format!(
        "{HEADER}{header}\n{PIXEL_DECLARATION}\n#line 1 \"pixel.hlsl\"\n{pixel_source}\n#line 1 \"rdi_wrapper.hlsl\"\n{footer}"
    );
    Ok(ShaderProgram {
        execution: None,
        vertex_bytecode: compile_stage(&vertex, true, s!("rdi_vertex"))?.into(),
        pixel_bytecode: compile_stage(&pixel, false, pixel_entry)?.into(),
        pipeline: source.pipeline,
    })
}

fn compile_stage(source: &str, vertex: bool, entry: PCSTR) -> Result<Vec<u8>, DesktopError> {
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    // SAFETY: source bytes outlive compilation; output slots are valid; no include handler is installed.
    let result = unsafe {
        D3DCompile(
            source.as_ptr().cast(),
            source.len(),
            s!("rdi.hlsl"),
            None,
            None,
            entry,
            if vertex { s!("vs_5_0") } else { s!("ps_5_0") },
            D3DCOMPILE_ENABLE_STRICTNESS | D3DCOMPILE_OPTIMIZATION_LEVEL3,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    if let Err(error) = result {
        let message = errors
            .as_ref()
            .map(|blob| String::from_utf8_lossy(&blob_bytes(blob)).into_owned())
            .unwrap_or_else(|| error.to_string());
        return Err(DesktopError::InvalidEffect(message));
    }
    code.as_ref()
        .map(blob_bytes)
        .ok_or_else(|| DesktopError::InvalidEffect("compiler returned no bytecode".into()))
}

fn blob_bytes(blob: &ID3DBlob) -> Vec<u8> {
    // SAFETY: the blob owns GetBufferSize initialized bytes until after the copy.
    unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize())
            .to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_compiles_and_preserves_aliases() {
        let mut names = std::collections::BTreeSet::new();
        for &builtin in BuiltinShader::ALL {
            assert!(names.insert(builtin.name()), "duplicate built-in name");
            assert_eq!(builtin.name().parse::<BuiltinShader>().unwrap(), builtin);
            let program = compile(builtin).unwrap();
            assert_eq!(program, compile(builtin.source()).unwrap());
            assert_eq!(program.pipeline, builtin.source().pipeline);
            assert!(!program.vertex_bytecode.is_empty());
            assert!(!program.pixel_bytecode.is_empty());
        }
        for (builtin, alias) in [
            (BuiltinShader::Identity, compile(IDENTITY).unwrap()),
            (BuiltinShader::Glitch, compile(GLITCH).unwrap()),
            (
                BuiltinShader::ParticleVortex,
                compile(PARTICLE_VORTEX).unwrap(),
            ),
        ] {
            assert_eq!(compile(builtin).unwrap(), alias);
        }
        assert!("unknown".parse::<BuiltinShader>().is_err());
    }

    #[test]
    fn rejects_incompatible_stage_entries() {
        for pipeline in [ShaderPipeline::Sprite, ShaderPipeline::Particles, ShaderPipeline::Procedural] {
            for vertex in [
                "float4 vertex(Instance instance, uint vertex_id : SV_VertexID) : SV_Position { return default_vertex(instance, vertex_id).position; }",
                "VertexOutput vertex(uint vertex_id : SV_VertexID) { return (VertexOutput)0; }",
            ] {
                assert!(
                    compile(ShaderSource {
                        execution: None,
                        pipeline,
                        vertex: Some(vertex),
                        pixel: None
                    })
                    .is_err()
                );
            }
            for pixel in [
                "float pixel(VertexOutput input) : SV_Target { return 1; }",
                "float4 pixel(float2 uv : TEXCOORD0) : SV_Target { return float4(uv, 0, 1); }",
            ] {
                assert!(
                    compile(ShaderSource {
                        execution: None,
                        pipeline,
                        vertex: None,
                        pixel: Some(pixel)
                    })
                    .is_err()
                );
            }
        }
    }

    #[test]
    fn compiles_runtime_programs_and_reports_source_errors() {
        assert!(compile(IDENTITY).is_ok());
        assert!(compile(GLITCH).is_ok());
        let particles = compile(PARTICLE_VORTEX).unwrap();
        assert!(!particles.vertex_bytecode.is_empty());
        assert_eq!(
            compile(GLITCH).unwrap(),
            compile(GLITCH.pixel.unwrap()).unwrap()
        );
        for pipeline in [ShaderPipeline::Sprite, ShaderPipeline::Particles, ShaderPipeline::Procedural] {
            for vertex in [None, Some(DEFAULT_VERTEX)] {
                for pixel in [None, Some(DEFAULT_PIXEL)] {
                    let program = compile(ShaderSource {
                        execution: None,
                        pipeline,
                        vertex,
                        pixel,
                    })
                    .unwrap();
                    assert_eq!(program.pipeline, pipeline);
                    assert!(!program.vertex_bytecode.is_empty());
                    assert!(!program.pixel_bytecode.is_empty());
                }
            }
        }
        assert!(
            compile(ShaderSource {
                execution: None,
                vertex: Some("invalid vertex"),
                ..PARTICLE_VORTEX
            })
            .unwrap_err()
            .to_string()
            .contains("vertex.hlsl")
        );
        assert!(
            compile(ShaderSource {
                execution: None,
                pixel: Some("invalid pixel"),
                ..PARTICLE_VORTEX
            })
            .unwrap_err()
            .to_string()
            .contains("pixel.hlsl")
        );
        assert!(
            compile(ShaderSource {
                execution: None,
                vertex: Some(&" ".repeat(1024 * 1024)),
                ..PARTICLE_VORTEX
            })
            .is_err()
        );
        assert!(
            compile("not valid hlsl")
                .unwrap_err()
                .to_string()
                .contains("pixel.hlsl")
        );
        assert!(compile("#include \"external.hlsl\"").is_err());
    }
}

# Build and Reuse Effects

Compile source with the Windows backend, then attach the resulting `Effect` to
an animation specification. These construction recipes do not move icons.
Read [shader concepts](../../concepts/shaders/index.md) for ownership and safety,
and [custom HLSL](../../concepts/shaders/custom.md) for stage/pass contracts.

## Built-in With Named Parameters

```rust
use std::collections::BTreeMap;
use rdi_core::{Curve, Effect, Keyframe, KeyframeInterp};
use rdi_platform_windows::shader::{self, BuiltinShader};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = shader::compile(BuiltinShader::SilkFlow)?
        .with_parameters(&BTreeMap::from([("strands".to_owned(), 8.0)]))?;
    let effect = Effect {
        params: program.default_params(), shader: program,
        padding_px: 24, seed: 7.0,
        envelope: Curve::keyframes(vec![
            Keyframe::new(0.0, 1.0), Keyframe::new(1.0, 1.0),
        ], KeyframeInterp::Linear)?,
    };
    effect.validate()?;
    Ok(())
}
```

## Custom Pixel Stage

```rust
use rdi_core::{Curve, Effect, Keyframe, KeyframeInterp};
use rdi_platform_windows::shader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = shader::compile(r#"
float4 pixel(VertexOutput input) : SV_Target {
    float4 color = default_pixel(input);
    color.rgb *= float3(0.4, 1.0, 0.7);
    return color;
}
"#)?;
    let effect = Effect {
        params: program.default_params(), shader: program,
        padding_px: 16, seed: 0.0,
        envelope: Curve::keyframes(vec![
            Keyframe::new(0.0, 0.0), Keyframe::new(0.5, 1.0),
            Keyframe::new(1.0, 0.0),
        ], KeyframeInterp::SmoothStep)?,
    };
    effect.validate()?;
    Ok(())
}
```

## Describe a Procedural Pass

```rust
use std::sync::Arc;
use rdi_core::{EffectParameter, ParameterKind, ShaderPipeline};
use rdi_platform_windows::shader::{self, ExecutionSource, PassSource, ShaderSource};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let recipe = ExecutionSource {
        parameters: vec![EffectParameter {
            name: "green".into(), kind: ParameterKind::Float,
            default: 0.8, min: 0.0, max: 1.0,
        }],
        passes: vec![PassSource {
            pixel: Some(r#"
float4 pixel(VertexOutput input) : SV_Target {
    float4 color = default_pixel(input);
    color.rgb *= float3(0.5, parameter(0), 0.8);
    return color;
}
"#.into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let program = shader::compile(ShaderSource {
        pipeline: ShaderPipeline::Procedural,
        execution: Some(Arc::new(recipe)), vertex: None, pixel: None,
    })?;
    println!("Pipeline: {:?}", program.pipeline);
    Ok(())
}
```

For extra passes, fill `ExecutionSource.targets` with `EffectTarget` values and
set each `PassSource.inputs`, `output`, `draw` and `blend`. Input indices refer
to preceding targets. The [Python two-pass recipe](../../python/guides/shaders.md)
shows the same graph and HLSL; the shared contract maps each descriptor to Rust.
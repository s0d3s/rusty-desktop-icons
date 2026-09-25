//! Built-in shader bodies, default stages and shared pipeline HLSL.

use crate::shader::ShaderSource;
use rdi_core::{AnimationPreset, Curve, DesktopError, Duration, Keyframe, KeyframeInterp, ShaderPipeline};
use std::str::FromStr;
use windows::core::{PCSTR, s};

macro_rules! builtin_shaders {
    (@preset) => { AnimationPreset::default() };
    (@preset $preset:expr) => { $preset };
    ($(
        $variant:ident {
            name: $name:literal,
            pipeline: $pipeline:ident,
            execution: $execution:expr,
            vertex: $vertex:expr,
            pixel: $pixel:expr $(, default_preset: $preset:expr)? $(,)?
        }
    ),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum BuiltinShader {
            $($variant),+
        }

        impl BuiltinShader {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// Return independent recommended values without compiling a shader.
            pub fn default_preset(self) -> AnimationPreset {
                match self {
                    $(Self::$variant => builtin_shaders!(@preset $($preset)?)),+
                }
            }

            pub fn source(self) -> ShaderSource<'static> {
                match self {
                    $(Self::$variant => ShaderSource {
                        execution: $execution,
                        pipeline: ShaderPipeline::$pipeline,
                        vertex: $vertex,
                        pixel: $pixel,
                    }),+
                }
            }

            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name),+
                }
            }
        }

        impl FromStr for BuiltinShader {
            type Err = DesktopError;

            fn from_str(name: &str) -> Result<Self, Self::Err> {
                match name {
                    $($name => Ok(Self::$variant),)+
                    _ => Err(DesktopError::InvalidEffect(format!(
                        "unknown built-in shader: {name}"
                    ))),
                }
            }
        }
    };
}

builtin_shaders! {
    Identity {
        name: "identity",
        pipeline: Sprite,
        execution: None,
        vertex: None,
        pixel: None,
    },
    Glitch {
        name: "glitch",
        pipeline: Sprite,
        execution: None,
        vertex: None,
        pixel: Some(GLITCH_PIXEL),
        default_preset: AnimationPreset {
            movement: Curve::keyframes(
                vec![Keyframe::new(0.0, 0.0), Keyframe::new(0.15, 0.0), Keyframe::new(0.85, 1.0), Keyframe::new(1.0, 1.0)],
                KeyframeInterp::SmoothStep,
            ).expect("valid glitch preset movement"),
            ..AnimationPreset::default()
        },
    },
    ParticleVortex {
        name: "particle-vortex",
        pipeline: Particles,
        execution: None,
        vertex: Some(PARTICLE_VORTEX_VERTEX),
        pixel: Some(PARTICLE_VORTEX_PIXEL),
        default_preset: AnimationPreset {
            duration: Duration::fixed(std::time::Duration::from_secs(4)),
            ..AnimationPreset::default()
        },
    },
    DustTransfer {
        name: "dust-transfer",
        pipeline: Particles,
        execution: None,
        vertex: Some(DUST_TRANSFER_VERTEX),
        pixel: Some(PARTICLE_VORTEX_PIXEL),
        default_preset: transport_preset(4),
    },
    SilkFlow {
        name: "silk-flow",
        pipeline: Procedural,
        execution: Some(std::sync::Arc::new(silk_flow_execution())),
        vertex: Some(SILK_FLOW),
        pixel: Some(SILK_FLOW),
        default_preset: transport_preset(5),
    },
}

fn transport_preset(seconds: u64) -> AnimationPreset {
    AnimationPreset {
        envelope: Curve::keyframes(
            vec![Keyframe::new(0.0, 1.0), Keyframe::new(1.0, 1.0)],
            KeyframeInterp::Linear,
        ).expect("valid transport preset envelope"),
        duration: Duration::fixed(std::time::Duration::from_secs(seconds)),
        ..AnimationPreset::default()
    }
}

impl<'a> From<BuiltinShader> for ShaderSource<'a> {
    fn from(shader: BuiltinShader) -> Self {
        shader.source()
    }
}

pub const IDENTITY: &str = DEFAULT_PIXEL;
pub const GLITCH: ShaderSource<'static> = ShaderSource {
    execution: None,
    pipeline: ShaderPipeline::Sprite,
    vertex: None,
    pixel: Some(GLITCH_PIXEL),
};
pub const PARTICLE_VORTEX: ShaderSource<'static> = ShaderSource {
    execution: None,
    pipeline: ShaderPipeline::Particles,
    vertex: Some(PARTICLE_VORTEX_VERTEX),
    pixel: Some(PARTICLE_VORTEX_PIXEL),
};

fn silk_flow_execution() -> crate::shader::ExecutionSource {
    use rdi_core::{DrawSpec, EffectParameter, ParameterKind};
    crate::shader::ExecutionSource {
        parameters: [
            ("strands", ParameterKind::Integer, 8.0, 2.0, 16.0),
            ("spread", ParameterKind::Float, 1.0, 0.0, 3.0),
            ("folds", ParameterKind::Float, 2.0, 0.25, 6.0),
            ("density", ParameterKind::Float, 0.8, 0.1, 1.0),
        ]
        .into_iter()
        .map(|(name, kind, default, min, max)| EffectParameter {
            name: name.into(),
            kind,
            default,
            min,
            max,
        })
        .collect(),
        passes: vec![crate::shader::PassSource {
            draw: DrawSpec {
                vertices: 24,
                parameter: Some(0),
                multiplier: 128 * 6 * 2,
                ..Default::default()
            },
            ..Default::default()
        }],
        body_artwork: true,
        label_artwork: true,
        ..Default::default()
    }
}

const SILK_FLOW: &str = include_str!("hlsl/effects/silk_flow.hlsl");

pub(crate) fn pipeline_template(pipeline: ShaderPipeline) -> (&'static str, &'static str, PCSTR) {
    match pipeline {
        ShaderPipeline::Sprite => (SPRITE_HEADER, SPRITE_FOOTER, s!("rdi_pixel")),
        ShaderPipeline::Particles => (PARTICLE_HEADER, PARTICLE_FOOTER, s!("rdi_pixel")),
        ShaderPipeline::Procedural => (
            include_str!("hlsl/pipelines/procedural.hlsl"),
            PARTICLE_FOOTER,
            s!("rdi_pixel"),
        ),
    }
}

/// Default vertex entry point; its helper is supplied by the selected pipeline.
pub const DEFAULT_VERTEX: &str = include_str!("hlsl/shared/default_vertex.hlsl");
/// Default pixel entry point; its helper is supplied by the selected pipeline.
pub const DEFAULT_PIXEL: &str = include_str!("hlsl/shared/default_pixel.hlsl");
pub(crate) const VERTEX_DECLARATION: &str = include_str!("hlsl/shared/vertex_declaration.hlsl");
pub(crate) const PIXEL_DECLARATION: &str = include_str!("hlsl/shared/pixel_declaration.hlsl");
pub(crate) const VERTEX_FOOTER: &str = include_str!("hlsl/shared/vertex_footer.hlsl");

pub(crate) const HEADER: &str = include_str!("hlsl/shared/header.hlsl");

const SPRITE_HEADER: &str = include_str!("hlsl/pipelines/sprite_header.hlsl");

const SPRITE_FOOTER: &str = include_str!("hlsl/shared/sprite_footer.hlsl");

const PARTICLE_HEADER: &str = include_str!("hlsl/pipelines/particle_header.hlsl");

const PARTICLE_FOOTER: &str = include_str!("hlsl/shared/particle_footer.hlsl");

/// Glitch displacement plus RGB split. params: displacement px, split px,
/// band height px, temporal frequency Hz. timing: elapsed, progress, strength, seed.
const GLITCH_PIXEL: &str = include_str!("hlsl/effects/glitch_pixel.hlsl");

const PARTICLE_VORTEX_VERTEX: &str = include_str!("hlsl/effects/particle_vortex_vertex.hlsl");

const PARTICLE_VORTEX_PIXEL: &str = include_str!("hlsl/effects/particle_vortex_pixel.hlsl");

const DUST_TRANSFER_VERTEX: &str = include_str!("hlsl/effects/dust_transfer_vertex.hlsl");

#[cfg(test)]
mod tests {
    use super::*;
    use rdi_core::{AnimationCurve, Effect, Point};

    #[test]
    fn animation_presets_cover_catalog_and_preserve_clean_endpoints() {
        for &shader in BuiltinShader::ALL {
            let preset = shader.default_preset();
            let seconds = match shader {
                BuiltinShader::Identity | BuiltinShader::Glitch => 2,
                BuiltinShader::ParticleVortex | BuiltinShader::DustTransfer => 4,
                BuiltinShader::SilkFlow => 5,
            };
            assert_eq!(preset.duration.resolve(Point::ZERO, Point::new(100, 50)).unwrap().as_secs(), seconds);
            assert_eq!(preset.movement.eval(0.0), 0.0);
            assert_eq!(preset.movement.eval(1.0), 1.0);
            for step in 0..=100 {
                let progress = step as f32 / 100.0;
                assert!(preset.movement.eval(progress).is_finite());
                assert!((0.0..=1.0).contains(&preset.envelope.eval(progress)));
            }
            let program = crate::shader::compile(shader).unwrap();
            let effect = Effect {
                params: program.default_params(), shader: program, padding_px: 16,
                envelope: preset.envelope, seed: 0.0,
            };
            effect.validate().unwrap();
            assert_eq!(effect.strength(0.0), 0.0);
            assert_eq!(effect.strength(1.0), 0.0);
            assert_eq!(effect.strength(0.5), 1.0);
        }
    }

    #[test]
    fn animation_presets_use_optional_fallback_and_independent_overrides() {
        let identity = BuiltinShader::Identity.default_preset();
        let fallback = AnimationPreset::default();
        assert_eq!(identity.movement, fallback.movement);
        assert_eq!(identity.envelope, fallback.envelope);
        assert_eq!(identity.duration, fallback.duration);
        let glitch = BuiltinShader::Glitch.default_preset();
        assert_eq!(glitch.movement.eval(0.15), 0.0);
        assert_eq!(glitch.movement.eval(0.85), 1.0);
        for shader in [BuiltinShader::DustTransfer, BuiltinShader::SilkFlow] {
            let mut preset = shader.default_preset();
            for progress in [0.0, 0.05, 0.5, 0.95, 1.0] {
                assert_eq!(preset.envelope.eval(progress), 1.0);
            }
            preset.envelope = Curve::linear();
            assert_eq!(shader.default_preset().envelope.eval(0.05), 1.0);
        }
    }
}

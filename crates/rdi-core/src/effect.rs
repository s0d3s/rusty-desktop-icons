//! Device-independent shader bytecode and per-icon visual state.

use crate::{AnimationCurve, Curve, DesktopError, IconId, Point};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderPipeline {
    Sprite,
    Particles,
    Procedural,
}

/// Compiled Windows vertex/pixel program. GPU objects belong to the renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderProgram {
    pub execution: Option<Arc<crate::EffectExecution>>,
    pub pixel_bytecode: Arc<[u8]>,
    pub vertex_bytecode: Arc<[u8]>,
    pub pipeline: ShaderPipeline,
}

impl ShaderProgram {
    pub fn default_params(&self) -> [f32; 4] {
        match self.pipeline {
            ShaderPipeline::Sprite => [8.0, 3.0, 6.0, 30.0],
            ShaderPipeline::Particles => [3.0, 1.5, 2.0, 0.65],
            ShaderPipeline::Procedural => self.execution.as_ref().map_or([0.0; 4], |execution| {
                let values = execution.defaults();
                [values[0], values[1], values[2], values[3]]
            }),
        }
    }

    pub fn with_parameters(&self, values: &std::collections::BTreeMap<String, f32>) -> Result<Self, DesktopError> {
        let mut result = self.clone();
        let execution = result.execution.as_mut().ok_or_else(|| DesktopError::InvalidEffect("named parameters require a procedural definition".into()))?;
        let execution = Arc::make_mut(execution);
        for (name, value) in values {
            let parameter = execution.parameters.iter_mut().find(|parameter| parameter.name == *name)
                .ok_or_else(|| DesktopError::InvalidEffect(format!("unknown parameter: {name}")))?;
            if !parameter.accepts(*value) { return Err(DesktopError::InvalidEffect(format!("invalid parameter: {name}"))); }
            parameter.default = *value;
        }
        execution.validate(&execution.defaults())?;
        Ok(result)
    }
}

/// Immutable effect configuration, shared by preparation and rendering.
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub shader: ShaderProgram,
    pub params: [f32; 4],
    pub padding_px: u32,
    pub envelope: Curve,
    pub seed: f32,
}

impl Effect {
    /// Validate resource bounds and constants before allocating GPU resources.
    pub fn validate(&self) -> Result<(), DesktopError> {
        if self.shader.pixel_bytecode.is_empty()
            || self.shader.vertex_bytecode.is_empty()
            || self.padding_px > 256
            || !self.seed.is_finite()
            || self.params.iter().any(|value| !value.is_finite())
        {
            return Err(DesktopError::InvalidEffect(
                "empty shader, non-finite constants or padding > 256".into(),
            ));
        }
        if self.shader.pipeline == ShaderPipeline::Particles
            && (!(1.0..=32.0).contains(&self.params[0])
                || !(0.0..=3.0).contains(&self.params[1])
                || !(-8.0..=8.0).contains(&self.params[2])
                || !(0.1..=2.0).contains(&self.params[3])
                || self.seed.abs() > 65535.0)
        {
            return Err(DesktopError::InvalidEffect(
                "particle pipeline requires cell_px in [1,32], radius in [0,3], turns in [-8,8], dust_size in [0.1,2], and abs(seed) <= 65535".into(),
            ));
        }
        if self.shader.pipeline == ShaderPipeline::Procedural {
            let execution = self.shader.execution.as_ref().ok_or_else(|| DesktopError::InvalidEffect("missing procedural execution".into()))?;
            execution.validate(&self.parameter_values())?;
            if self.seed.abs() > 65535.0 { return Err(DesktopError::InvalidEffect("abs(seed) exceeds 65535".into())); }
        } else if self.shader.execution.is_some() {
            return Err(DesktopError::InvalidEffect("execution requires procedural pipeline".into()));
        }
        Ok(())
    }

    pub fn parameter_values(&self) -> [f32; 16] {
        let mut values = self.shader.execution.as_ref().map_or([0.0; 16], |execution| execution.defaults());
        values[..4].copy_from_slice(&self.params);
        values
    }

    /// Endpoints are always identity, regardless of the supplied curve.
    pub fn strength(&self, progress: f32) -> f32 {
        if progress <= 0.0 || progress >= 1.0 {
            0.0
        } else {
            self.envelope.eval(progress).clamp(0.0, 1.0)
        }
    }
}

/// Per-frame state; no shader compilation or texture data travels here.
#[derive(Clone, Debug)]
pub struct IconFrame {
    pub id: IconId,
    pub position: Point,
    pub progress: f32,
    pub elapsed_seconds: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procedural_requires_execution() {
        let shader = ShaderProgram {
            execution: None,
            pixel_bytecode: Arc::from([1u8]),
            vertex_bytecode: Arc::from([1u8]),
            pipeline: ShaderPipeline::Procedural,
        };
        let effect = Effect {
            params: shader.default_params(), shader, padding_px: 16,
            envelope: Curve::linear(), seed: 3.0,
        };
        assert!(effect.validate().is_err());
        assert_eq!(effect.strength(0.0), 0.0);
        assert_eq!(effect.strength(1.0), 0.0);
    }

    #[test]
    fn particle_parameters_are_bounded() {
        let shader = ShaderProgram {
            execution: None,
            pixel_bytecode: Arc::from([1u8]),
            vertex_bytecode: Arc::from([1u8]),
            pipeline: ShaderPipeline::Particles,
        };
        let mut effect = Effect {
            params: shader.default_params(),
            shader,
            padding_px: 0,
            envelope: Curve::linear(),
            seed: 0.0,
        };
        effect.validate().unwrap();
        for (index, value) in [
            (0, 0.0),
            (0, 33.0),
            (1, -1.0),
            (1, 4.0),
            (2, -9.0),
            (2, 9.0),
            (3, 0.0),
            (3, 3.0),
        ] {
            let mut invalid = effect.clone();
            invalid.params[index] = value;
            assert!(invalid.validate().is_err());
        }
        effect.seed = f32::MAX;
        assert!(effect.validate().is_err());
    }

    #[test]
    fn envelope_endpoints_are_identity() {
        let effect = Effect {
            shader: ShaderProgram {
                execution: None,
                pixel_bytecode: Arc::from([1u8]),
                vertex_bytecode: Arc::from([1u8]),
                pipeline: ShaderPipeline::Sprite,
            },
            params: [0.0; 4],
            padding_px: 16,
            envelope: Curve::linear(),
            seed: 0.0,
        };
        assert!(effect.validate().is_ok());
        assert_eq!(effect.strength(0.0), 0.0);
        assert_eq!(effect.strength(0.5), 0.5);
        assert_eq!(effect.strength(1.0), 0.0);
        let invalid = Effect {
            padding_px: 257,
            ..effect
        };
        assert!(invalid.validate().is_err());
    }
}

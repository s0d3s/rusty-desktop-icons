//! Validated, stateless procedural render recipes.

use crate::DesktopError;
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterKind {
    Float,
    Integer,
    Boolean,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectParameter {
    pub name: String,
    pub kind: ParameterKind,
    pub default: f32,
    pub min: f32,
    pub max: f32,
}

impl EffectParameter {
    pub fn accepts(&self, value: f32) -> bool {
        value.is_finite()
            && value >= self.min
            && value <= self.max
            && match self.kind {
                ParameterKind::Float => true,
                ParameterKind::Integer => value.fract() == 0.0,
                ParameterKind::Boolean => value == 0.0 || value == 1.0,
            }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawTopology {
    Triangles,
    TriangleStrip,
    Lines,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassBlend {
    Over,
    Add,
    Replace,
}

impl Default for PassBlend {
    fn default() -> Self {
        Self::Over
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawSpec {
    pub vertices: u32,
    pub parameter: Option<usize>,
    pub multiplier: u32,
    pub topology: DrawTopology,
}

impl Default for DrawSpec {
    fn default() -> Self {
        Self {
            vertices: 6,
            parameter: None,
            multiplier: 0,
            topology: DrawTopology::Triangles,
        }
    }
}

impl DrawSpec {
    pub fn count(&self, values: &[f32]) -> Result<u32, DesktopError> {
        let additional = match self.parameter {
            Some(index) => {
                let value = *values
                    .get(index)
                    .ok_or_else(|| invalid("draw parameter is missing"))?;
                if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value > 65536.0 {
                    return Err(invalid(
                        "draw parameter must be a bounded nonnegative integer",
                    ));
                }
                (value as u32)
                    .checked_mul(self.multiplier)
                    .ok_or_else(|| invalid("draw overflow"))?
            }
            None => 0,
        };
        let count = self
            .vertices
            .checked_add(additional)
            .ok_or_else(|| invalid("draw overflow"))?;
        if count == 0
            || count > 65536
            || match self.topology {
                DrawTopology::Triangles => count % 3 != 0,
                DrawTopology::TriangleStrip => count < 3,
                DrawTopology::Lines => count % 2 != 0,
            }
        {
            return Err(invalid("invalid topology or vertex count (limit 65536)"));
        }
        Ok(count)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectTarget {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectPass {
    pub vertex_bytecode: Arc<[u8]>,
    pub pixel_bytecode: Arc<[u8]>,
    pub draw: DrawSpec,
    pub inputs: Vec<usize>,
    pub output: Option<usize>,
    pub blend: PassBlend,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectExecution {
    pub parameters: Vec<EffectParameter>,
    pub targets: Vec<EffectTarget>,
    pub passes: Vec<EffectPass>,
    pub body_artwork: bool,
    pub label_artwork: bool,
}

fn invalid(message: &str) -> DesktopError {
    DesktopError::InvalidEffect(message.into())
}

impl EffectExecution {
    pub fn defaults(&self) -> [f32; 16] {
        let mut values = [0.0; 16];
        for (value, parameter) in values.iter_mut().zip(&self.parameters) {
            *value = parameter.default;
        }
        values
    }

    pub fn target_bytes(&self) -> u64 {
        self.targets
            .iter()
            .map(|target| target.width as u64 * target.height as u64 * 4)
            .sum()
    }

    pub fn validate(&self, values: &[f32]) -> Result<(), DesktopError> {
        if self.parameters.len() > 16
            || self.targets.len() > 4
            || self.passes.is_empty()
            || self.passes.len() > 8
        {
            return Err(invalid(
                "procedural limits: 16 parameters, 4 targets, 1..8 passes",
            ));
        }
        let mut names = HashSet::new();
        for (index, parameter) in self.parameters.iter().enumerate() {
            if parameter.name.is_empty()
                || !parameter
                    .name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                || !names.insert(&parameter.name)
                || !parameter.min.is_finite()
                || !parameter.max.is_finite()
                || parameter.min > parameter.max
                || !parameter.accepts(parameter.min)
                || !parameter.accepts(parameter.max)
                || !parameter.accepts(parameter.default)
                || !values
                    .get(index)
                    .is_some_and(|value| parameter.accepts(*value))
            {
                return Err(invalid("invalid parameter schema or value"));
            }
        }
        if self.targets.iter().any(|target| {
            target.width == 0 || target.height == 0 || target.width > 8192 || target.height > 8192
        }) || self.target_bytes() > 64 * 1024 * 1024
        {
            return Err(invalid(
                "intermediate targets exceed dimensions or 64 MiB budget",
            ));
        }
        let mut written = HashSet::new();
        for (index, pass) in self.passes.iter().enumerate() {
            if pass.vertex_bytecode.is_empty()
                || pass.pixel_bytecode.is_empty()
                || pass.inputs.len() > 4
                || pass.inputs.iter().any(|input| !written.contains(input))
            {
                return Err(invalid(
                    "empty stages or input target has no preceding producer",
                ));
            }
            if let Some(parameter) = pass.draw.parameter {
                let schema = self
                    .parameters
                    .get(parameter)
                    .ok_or_else(|| invalid("unknown draw parameter"))?;
                if schema.kind != ParameterKind::Integer || schema.min < 0.0 {
                    return Err(invalid(
                        "draw counts require a nonnegative integer parameter",
                    ));
                }
            }
            pass.draw.count(values)?;
            let mut maximum = self.defaults();
            if let Some(parameter) = pass.draw.parameter {
                maximum[parameter] = self.parameters[parameter].max;
            }
            pass.draw.count(&maximum)?;
            if let Some(parameter) = pass.draw.parameter {
                maximum[parameter] = self.parameters[parameter].min;
                pass.draw.count(&maximum)?;
                if self.parameters[parameter].min < self.parameters[parameter].max {
                    maximum[parameter] += 1.0;
                    pass.draw.count(&maximum)?;
                }
            }
            if let Some(target) = pass.output {
                if target >= self.targets.len()
                    || pass.inputs.contains(&target)
                    || !written.insert(target)
                {
                    return Err(invalid(
                        "target must have one producer and cannot be read while written",
                    ));
                }
            } else if index + 1 != self.passes.len() {
                return Err(invalid("only the last pass may composite to the scene"));
            }
        }
        if self.passes.last().unwrap().output.is_some() || written.len() != self.targets.len() {
            return Err(invalid(
                "graph must end at the scene and produce every declared target",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn graph() -> EffectExecution {
        let pass = EffectPass {
            vertex_bytecode: Arc::from([1]),
            pixel_bytecode: Arc::from([1]),
            draw: DrawSpec::default(),
            inputs: vec![],
            output: Some(0),
            blend: PassBlend::Over,
        };
        EffectExecution {
            targets: vec![EffectTarget {
                width: 32,
                height: 32,
            }],
            passes: vec![
                pass.clone(),
                EffectPass {
                    inputs: vec![0],
                    output: None,
                    ..pass
                },
            ],
            ..Default::default()
        }
    }
    #[test]
    fn validates_graph_order_and_budgets() {
        let valid = graph();
        valid.validate(&[]).unwrap();
        let mut invalid = valid.clone();
        invalid.passes[0].inputs = vec![0];
        assert!(invalid.validate(&[]).is_err());
        let mut invalid = valid.clone();
        invalid.passes[1].output = Some(0);
        assert!(invalid.validate(&[]).is_err());
        let mut invalid = valid.clone();
        invalid.targets[0].width = 8193;
        assert!(invalid.validate(&[]).is_err());
        let mut invalid = valid;
        invalid.passes[0].draw.vertices = 65538;
        assert!(invalid.validate(&[]).is_err());
    }

    #[test]
    fn validates_parameters_and_draw_ranges() {
        let mut recipe = graph();
        recipe.parameters = vec![EffectParameter {
            name: "count".into(),
            kind: ParameterKind::Integer,
            default: 2.0,
            min: 1.0,
            max: 8.0,
        }];
        recipe.passes[0].draw = DrawSpec {
            vertices: 0,
            parameter: Some(0),
            multiplier: 6,
            ..Default::default()
        };
        recipe.validate(&[2.0]).unwrap();
        for value in [0.0, 9.0, 2.5, f32::NAN, f32::INFINITY] {
            assert!(recipe.validate(&[value]).is_err());
        }
        let mut invalid = recipe.clone();
        invalid.parameters.push(invalid.parameters[0].clone());
        assert!(invalid.validate(&[2.0, 2.0]).is_err());
        let mut invalid = recipe.clone();
        invalid.parameters[0].max = 8.5;
        assert!(invalid.validate(&[2.0]).is_err());
        let mut invalid = recipe.clone();
        invalid.passes[0].draw.multiplier = 65536;
        assert!(invalid.validate(&[2.0]).is_err());
        let mut invalid = recipe.clone();
        invalid.parameters[0].kind = ParameterKind::Float;
        assert!(invalid.validate(&[2.0]).is_err());
        let mut invalid = recipe;
        invalid.passes[0].draw.multiplier = 1;
        assert!(invalid.validate(&[6.0]).is_err());
        let mut invalid = graph();
        invalid.targets[0] = EffectTarget {
            width: 8192,
            height: 8192,
        };
        assert!(invalid.validate(&[]).is_err());
        let mut invalid = graph();
        invalid.passes[1].inputs = vec![1];
        assert!(invalid.validate(&[]).is_err());
    }
}

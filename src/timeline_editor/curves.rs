use rdi_core::{AnimationCurve, Curve, Keyframe, KeyframeInterp};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub interp: String,
    pub keyframes: Vec<[f32; 2]>,
}

impl Track {
    pub fn from_curve(curve: &Curve, samples: usize) -> Self {
        match curve {
            Curve::Keyframe(keys) => Self {
                interp: match keys.interpolation() {
                    KeyframeInterp::Linear => "linear",
                    KeyframeInterp::Step => "step",
                    KeyframeInterp::SmoothStep => "smooth_step",
                }
                .into(),
                keyframes: keys.keys().iter().map(|key| [key.t, key.v]).collect(),
            },
            _ => {
                let samples = samples.clamp(2, 256);
                Self {
                    interp: "linear".into(),
                    keyframes: (0..samples)
                        .map(|index| {
                            let time = index as f32 / (samples - 1) as f32;
                            [time, curve.eval(time)]
                        })
                        .collect(),
                }
            }
        }
    }

    pub fn curve(&self, envelope: bool) -> Result<Curve, String> {
        if self.keyframes.len() > 4096 {
            return Err("At most 4096 keyframes".into());
        }
        if envelope
            && self
                .keyframes
                .iter()
                .any(|key| !(0.0..=1.0).contains(&key[1]))
        {
            return Err("Envelope values must be in [0, 1]".into());
        }
        let interpolation = match self.interp.as_str() {
            "linear" => KeyframeInterp::Linear,
            "step" => KeyframeInterp::Step,
            "smooth_step" => KeyframeInterp::SmoothStep,
            _ => return Err("Interpolation: linear, step or smooth_step".into()),
        };
        Curve::keyframes(
            self.keyframes
                .iter()
                .map(|key| Keyframe::new(key[0], key[1]))
                .collect(),
            interpolation,
        )
        .map_err(|error| error.to_string())
    }

    pub fn json(&self) -> String {
        let interp = serde_json::to_string(&self.interp).expect("interpolation string");
        let keys = self.keyframes.iter()
            .map(|key| serde_json::to_string(key).expect("finite keyframe"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{{\n    \"interp\":{interp},\n    \"keyframes\":[\n{keys}\n    ]\n}}")
    }

    pub fn parse(text: &str, envelope: bool) -> Result<Self, String> {
        if text.len() > 256 * 1024 {
            return Err("Curve JSON exceeds 256 KiB".into());
        }
        let track: Self = serde_json::from_str(text).map_err(|error| error.to_string())?;
        track.curve(envelope)?;
        Ok(track)
    }

    pub fn translated(
        &self,
        selected: &[usize],
        delta: [f32; 2],
        envelope: bool,
    ) -> Result<Self, String> {
        let mut result = self.clone();
        let fixed_time =
            selected.contains(&0) || selected.contains(&self.keyframes.len().saturating_sub(1));
        for &index in selected {
            let key = result
                .keyframes
                .get_mut(index)
                .ok_or("Selection no longer exists")?;
            if !fixed_time {
                key[0] += delta[0];
            }
            key[1] += delta[1];
        }
        result.curve(envelope)?;
        Ok(result)
    }

    pub fn delete(&self, selected: &[usize], envelope: bool) -> Result<Self, String> {
        if selected.contains(&0) || selected.contains(&(self.keyframes.len() - 1)) {
            return Err("Endpoint keyframes cannot be deleted".into());
        }
        let mut result = self.clone();
        result.keyframes = self
            .keyframes
            .iter()
            .enumerate()
            .filter(|(index, _)| !selected.contains(index))
            .map(|(_, key)| *key)
            .collect();
        result.curve(envelope)?;
        Ok(result)
    }

    pub fn insert(&self, key: [f32; 2], envelope: bool) -> Result<(Self, usize), String> {
        let index = self
            .keyframes
            .partition_point(|existing| existing[0] < key[0]);
        let mut result = self.clone();
        result.keyframes.insert(index, key);
        result.curve(envelope)?;
        Ok((result, index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_json_and_conversion_preserve_keyframes() {
        let curve = Curve::keyframes(
            vec![
                Keyframe::new(0.0, 0.0),
                Keyframe::new(0.123456, 0.75),
                Keyframe::new(1.0, 1.0),
            ],
            KeyframeInterp::Step,
        )
        .unwrap();
        let track = Track::from_curve(&curve, 33);
        assert_eq!(track.json(), "{\n    \"interp\":\"step\",\n    \"keyframes\":[\n[0.0,0.0], [0.123456,0.75], [1.0,1.0]\n    ]\n}");
        assert_eq!(
            Track::parse(&track.json(), false)
                .unwrap()
                .curve(false)
                .unwrap(),
            curve
        );
        let sampled = Track::from_curve(&Curve::ease_in_out(), 33);
        assert_eq!(sampled.keyframes.len(), 33);
        for key in sampled.keyframes {
            assert_eq!(key[1], Curve::ease_in_out().eval(key[0]));
        }
    }

    #[test]
    fn group_edits_preserve_spacing_and_endpoint_times() {
        let track = Track {
            interp: "linear".into(),
            keyframes: vec![[0.0, 0.0], [0.2, 0.3], [0.4, 0.5], [1.0, 1.0]],
        };
        let moved = track.translated(&[1, 2], [0.1, 0.2], false).unwrap();
        assert!((moved.keyframes[2][0] - moved.keyframes[1][0] - 0.2).abs() < 1e-6);
        assert_eq!(
            track.translated(&[0], [0.1, 0.2], false).unwrap().keyframes[0][0],
            0.0
        );
        assert_eq!(track.translated(&[0, 1], [0.1, 0.0], false).unwrap(), track);
        assert!(track.translated(&[1], [0.3, 0.0], false).is_err());
        assert!(track.delete(&[0], false).is_err());
        assert!(track.insert([0.2, 0.4], false).is_err());
        assert!(track.insert([0.3, 1.1], true).is_err());
        assert!(track.insert([0.3, 1.1], false).is_ok());
        assert!(
            Track::parse(
                r#"{"interp":"linear","keyframes":[[0,0],[1,1]],"typo":1}"#,
                false
            )
            .is_err()
        );
    }
}

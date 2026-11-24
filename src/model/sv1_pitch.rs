use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Sv1Pitch {
    pub generated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition_offset: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition_left: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition_right: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth_left: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth_right: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vibrato_delay: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vibrato_fade_in: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vibrato_fade_out: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vibrato_depth: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vibrato_phase: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vibrato_frequency: Option<f64>,
}

impl Sv1Pitch {
    pub fn valid(self) -> bool {
        [
            self.transition_offset,
            self.depth_left,
            self.depth_right,
            self.vibrato_phase,
        ]
        .into_iter()
        .flatten()
        .all(|value| (value as f32).is_finite())
            && [
                self.transition_left,
                self.transition_right,
                self.vibrato_delay,
                self.vibrato_fade_in,
                self.vibrato_fade_out,
                self.vibrato_depth,
                self.vibrato_frequency,
            ]
            .into_iter()
            .flatten()
            .all(|value| value >= 0.0 && (value as f32).is_finite())
    }
}

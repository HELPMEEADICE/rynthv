mod edit;
mod interpolation;
mod parameter;
mod sampling;
mod target;

pub use interpolation::Interpolation;
pub use parameter::Parameter;
pub use sampling::AutomationSampler;
pub use target::AutomationTarget;

use super::Extra;
use crate::tr;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Automation {
    pub mode: String,
    #[serde(with = "flat_points")]
    pub points: Vec<(i64, f64)>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for Automation {
    fn default() -> Self {
        Self {
            mode: Interpolation::Cubic.key().into(),
            points: Vec::new(),
            extra: Extra::new(),
        }
    }
}

impl Automation {
    pub fn interpolation(&self) -> Result<Interpolation, String> {
        Interpolation::parse(&self.mode)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.interpolation()?;
        validate_points(&self.points)
    }

    pub fn sampler(&self, default: f64) -> Result<AutomationSampler<'_>, String> {
        AutomationSampler::new(self, default)
    }

    pub fn sample(&self, position: i64, default: f64) -> Result<f64, String> {
        self.sampler(default)?.sample(position)
    }
}

fn validate_points(points: &[(i64, f64)]) -> Result<(), String> {
    if points.iter().any(|(_, value)| !(*value as f32).is_finite()) {
        return Err(tr!("参数控制点的值必须为有限数").into());
    }
    if points.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
        return Err(tr!("参数控制点的时间必须严格递增").into());
    }
    Ok(())
}

pub(super) mod flat_points {
    use crate::tr;
    use serde::{
        Deserializer, Serializer,
        de::{Error, SeqAccess, Visitor},
        ser::SerializeSeq,
    };
    use std::fmt;

    pub fn serialize<S: Serializer>(
        points: &[(i64, f64)],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(points.len() * 2))?;
        for &(time, value) in points {
            seq.serialize_element(&time)?;
            seq.serialize_element(&value)?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<(i64, f64)>, D::Error> {
        struct FlatPointsVisitor;

        impl<'de> Visitor<'de> for FlatPointsVisitor {
            type Value = Vec<(i64, f64)>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("flat array of [time, value, ...] pairs")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut points = Vec::with_capacity(seq.size_hint().unwrap_or(0) / 2);
                while let Some(time) = seq.next_element::<i64>()? {
                    let Some(value) = seq.next_element::<f64>()? else {
                        return Err(A::Error::custom(tr!("参数控制点必须为 time/value 对")));
                    };
                    if !value.is_finite() {
                        return Err(A::Error::custom(tr!("控制点值必须为有限数")));
                    }
                    points.push((time, value));
                }
                Ok(points)
            }
        }

        deserializer.deserialize_seq(FlatPointsVisitor)
    }
}

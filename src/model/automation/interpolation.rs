use crate::{tr, trf};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Interpolation {
    Linear,
    Cosine,
    Cubic,
}

impl Interpolation {
    pub const ALL: [Self; 3] = [Self::Linear, Self::Cosine, Self::Cubic];

    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "linear" => Ok(Self::Linear),
            "cosine" => Ok(Self::Cosine),
            "cubic" => Ok(Self::Cubic),
            _ => Err(trf!("未知参数插值模式：{}", value)),
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Cosine => "cosine",
            Self::Cubic => "cubic",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => tr!("线性"),
            Self::Cosine => tr!("余弦"),
            Self::Cubic => tr!("三次样条"),
        }
    }
}

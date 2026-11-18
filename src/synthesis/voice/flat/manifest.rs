use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{super::{Result, VoiceError, hex}, embedding};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRole {
    Acoustic,
    Vocoder,
    Duration,
    F0,
    SingingAssistant,
}

impl ModelRole {
    pub const fn directory(self) -> &'static str {
        match self {
            Self::Acoustic => "acoustic",
            Self::Vocoder => "vocoder",
            Self::Duration => "duration",
            Self::F0 => "f0",
            Self::SingingAssistant => "sing",
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PackageConfig {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub profile: bool,
    #[serde(default)]
    pub background: bool,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub model_types: Vec<String>,
    #[serde(default)]
    pub phonesets: Vec<String>,
    #[serde(flatten)]
    pub additional: BTreeMap<String, Value>,
}

/// Both field layouts are kept visible in inspection output. In particular
/// the documented singing assistant has a separate role from the vocoder.
#[derive(Debug, Deserialize, Serialize)]
pub struct FlatManifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub author: String,
    pub language: String,
    pub phoneset: Option<String>,
    #[serde(default)]
    pub support_languages: Vec<String>,
    pub acoustic: Option<String>,
    pub base_model: Option<String>,
    pub vocoder: Option<String>,
    pub sing_model: Option<String>,
    pub duration: Option<String>,
    pub timing_model: Option<String>,
    pub f0: Option<String>,
    pub f0_model: Option<String>,
    pub pitch: Option<String>,
    pub f0_style: Option<String>,
    pub timing: Option<String>,
    pub duration_style: Option<String>,
    pub format_version: Option<Value>,
    #[serde(default)]
    pub styles: Vec<FlatStyle>,
    #[serde(flatten)]
    pub additional: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FlatStyle {
    pub name: String,
    pub data: String,
    pub extra: Option<Value>,
    #[serde(flatten)]
    pub additional: BTreeMap<String, Value>,
}

impl FlatManifest {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
        let manifest: Self = serde_json::from_slice(bytes)
            .map_err(|error| VoiceError::new(format!("Invalid Flat NOFS-JSON: {error}")))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn database_id(&self) -> String {
        let mut digest = Sha256::new();
        for field in [&self.vendor, &self.name] {
            digest.update((field.len() as u64).to_le_bytes());
            digest.update(field.as_bytes());
        }
        format!("flat:{}", hex(&digest.finalize()))
    }

    pub fn revision(&self) -> Result<u32> {
        self.version
            .parse()
            .map_err(|_| VoiceError::new("Flat voice version must be a decimal u32"))
    }

    pub fn model_ids(&self) -> Result<BTreeMap<ModelRole, &str>> {
        [
            (ModelRole::Acoustic, alias(&self.acoustic, &self.base_model)?),
            (ModelRole::Vocoder, self.vocoder.as_deref()),
            (ModelRole::Duration, alias(&self.duration, &self.timing_model)?),
            (ModelRole::F0, alias(&self.f0, &self.f0_model)?),
            (ModelRole::SingingAssistant, self.sing_model.as_deref()),
        ]
        .into_iter()
        .filter_map(|(role, value)| value.map(|id| (role, id)))
        .map(|(role, id)| {
            validate_id(id)?;
            Ok((role, id))
        })
        .collect()
    }

    pub fn pitch_embedding(&self) -> Result<Option<[f32; 32]>> {
        alias(&self.f0_style, &self.pitch)?.map(embedding::decode).transpose()
    }

    pub fn timing_embedding(&self) -> Result<Option<[f32; 128]>> {
        alias(&self.duration_style, &self.timing)?.map(embedding::decode).transpose()
    }

    fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() || self.language.trim().is_empty() {
            return Err(VoiceError::new("Flat voice name or language is empty"));
        }
        self.revision()?;
        self.model_ids()?;
        self.pitch_embedding()?;
        self.timing_embedding()?;
        let mut names = BTreeSet::new();
        for style in &self.styles {
            if style.name.trim().is_empty()
                || (!names.insert(&style.name) && style.name != "(auxiliary)")
            {
                return Err(VoiceError::new("Flat style names are empty or ambiguous"));
            }
            style.embedding()?;
            style.extra_value()?;
        }
        Ok(())
    }
}

impl FlatStyle {
    pub fn embedding(&self) -> Result<[f32; 32]> {
        embedding::decode(&self.data)
            .map_err(|error| VoiceError::new(format!("Flat style {:?}: {error}", self.name)))
    }

    pub fn extra_value(&self) -> Result<Option<f32>> {
        match &self.extra {
            None => Ok(None),
            Some(Value::String(value)) => Ok(Some(embedding::decode::<1>(value)?[0])),
            Some(Value::Number(value)) => value
                .as_f64()
                .map(|scalar| scalar as f32)
                .filter(|scalar| scalar.is_finite())
                .map(Some)
                .ok_or_else(|| VoiceError::new("Invalid Flat style extra scalar")),
            _ => Err(VoiceError::new("Invalid Flat style extra scalar")),
        }
    }
}

pub(super) fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 128
        || !id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(VoiceError::new(format!("Invalid Flat model identifier {id:?}")));
    }
    Ok(())
}

fn alias<'a>(left: &'a Option<String>, right: &'a Option<String>) -> Result<Option<&'a str>> {
    match (left.as_deref(), right.as_deref()) {
        (Some(left), Some(right)) if left != right => {
            Err(VoiceError::new("Conflicting Flat NOFS-JSON field aliases"))
        }
        (left, right) => Ok(left.or(right)),
    }
}

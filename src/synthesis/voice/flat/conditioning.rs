use std::sync::Arc;

use crate::synthesis::{
    stages::timbre::TimbreStyles,
    voice::{PhoneSet, Result, VoiceError},
};

use super::FlatManifest;

pub(crate) struct FlatConditioning {
    pub base: [f32; 32],
    pub styles: TimbreStyles,
    pub pitch: Option<[f32; 32]>,
    pub timing: Option<[f32; 128]>,
    pub phones: Vec<PhoneSet>,
}

impl FlatConditioning {
    pub fn from_manifest(manifest: &FlatManifest) -> Result<(Self, Vec<String>)> {
        let mut base = None;
        let mut neutral: Arc<[f32]> = Arc::from([0.0; 32]);
        let mut variants = Vec::new();
        let mut names = Vec::new();
        let mut anonymous = 0;
        for style in &manifest.styles {
            let vector = style.embedding()?;
            match style.name.as_str() {
                "(base)" => base = Some(vector),
                "(default)" => neutral = Arc::from(vector),
                name => {
                    let name = if name == "(auxiliary)" {
                        loop {
                            anonymous += 1;
                            let candidate = format!("Auxiliary {anonymous}");
                            if !manifest.styles.iter().any(|style| style.name == candidate) {
                                break candidate;
                            }
                        }
                    } else {
                        name.to_owned()
                    };
                    variants.push(Arc::from(vector));
                    names.push(name);
                }
            }
        }
        Ok((
            Self {
                base: base.ok_or_else(|| {
                    VoiceError::new("Flat voice does not declare a (base) embedding")
                })?,
                styles: TimbreStyles { neutral, variants },
                pitch: manifest.pitch_embedding()?,
                timing: manifest.timing_embedding()?,
                phones: Vec::new(),
            },
            names,
        ))
    }
}

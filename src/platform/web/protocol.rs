use crate::synthesis::{
    inference::compute::Status,
    rendered::{PitchTrack, RenderedPhonemeTrack},
    settings::{SynthesisSettings, VoiceSelection},
    voice::{VoiceCatalog, VoiceConfiguration},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request {
    Scan,
    ImportVoices {
        paths: Vec<PathBuf>,
    },
    ImportDictionaries {
        path: PathBuf,
    },
    Settings {
        settings: SynthesisSettings,
    },
    RemoveVoice {
        selection: VoiceSelection,
    },
    Inventory {
        selection: VoiceSelection,
    },
    Context {
        name: String,
        version: u8,
        file: Option<PathBuf>,
        identity: Option<String>,
    },
    Midi {
        path: PathBuf,
    },
    Audio {
        path: PathBuf,
    },
    Preview {
        project: String,
        position: f64,
    },
    Export {
        project: String,
    },
}

#[derive(Serialize, Deserialize)]
pub struct Library {
    pub settings: SynthesisSettings,
    pub catalog: VoiceCatalog,
    pub warning: Option<String>,
    pub message: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Inventory {
    pub configuration: Option<VoiceConfiguration>,
    pub flat_warnings: Vec<String>,
    pub content_hash: String,
    pub file_count: usize,
    pub resource_count: usize,
    pub graph_count: usize,
    pub tensor_count: usize,
    pub needs_context: bool,
    pub elapsed_ms: f64,
}

#[derive(Serialize, Deserialize)]
pub struct Visuals {
    pub group: String,
    pub pitch: PitchTrack,
    pub phones: RenderedPhonemeTrack,
    pub note_ids: BTreeMap<u64, String>,
    pub origin: f64,
}

#[derive(Serialize, Deserialize)]
pub struct Preview {
    pub duration: Option<f64>,
    pub rendered: usize,
    pub reused: usize,
    pub cache_bytes: usize,
    pub diagnostics: Option<ComputeDiagnostics>,
}

#[derive(Serialize, Deserialize)]
pub struct ComputeDiagnostics {
    pub actual: String,
    pub status: Status,
    pub activity: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Progress { stage: String },
    Chunk { group: String, origin: f64 },
    Mix { origin: f64 },
    Visuals(Box<Visuals>),
    Audio { channels: usize, rate: u32 },
}

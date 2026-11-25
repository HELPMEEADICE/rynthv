use crate::synthesis::{
    inference::compute::{AdapterInfo, Diagnostics, Status},
    offline::PreviewMetrics,
    render_cache::CacheStats,
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
    pub adapters: Vec<AdapterInfo>,
    pub diagnostics: ComputeDiagnostics,
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
    pub frames: usize,
}

#[derive(Serialize, Deserialize)]
pub struct Preview {
    pub metrics: PreviewMetrics,
    pub cache_bytes: usize,
    pub diagnostics: ComputeDiagnostics,
}

#[derive(Serialize, Deserialize)]
pub struct Export {
    pub paths: Vec<PathBuf>,
    pub metrics: ExportMetrics,
    pub diagnostics: ComputeDiagnostics,
}

#[derive(Serialize, Deserialize)]
pub struct ExportMetrics {
    pub render_ms: f64,
    pub mix_ms: f64,
    pub wav_ms: f64,
    pub worker_ms: f64,
}

#[derive(Serialize, Deserialize)]
pub struct ComputeDiagnostics {
    pub actual: String,
    pub status: Status,
    pub activity: serde_json::Value,
}

impl TryFrom<Diagnostics> for ComputeDiagnostics {
    type Error = String;

    fn try_from(diagnostics: Diagnostics) -> Result<Self, Self::Error> {
        Ok(Self {
            actual: diagnostics.actual.into(),
            status: diagnostics.status,
            activity: serde_json::to_value(diagnostics.activity)
                .map_err(|error| error.to_string())?,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Progress {
        stage: String,
    },
    Counts(CacheStats),
    Layout {
        frames: usize,
        occupied: Vec<[usize; 2]>,
    },
    Block {
        index: usize,
        first: bool,
    },
    Visuals(Box<Visuals>),
    Audio {
        channels: usize,
        rate: u32,
        frames: usize,
    },
}

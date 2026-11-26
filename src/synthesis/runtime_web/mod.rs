//! Browser editor state backed by the resource and synthesis Worker.

mod context;
mod library;
mod preview;
mod transport;

pub use super::context_dialog::{
    ContextImport, ContextImportDialog, ContextImportPhase, ContextImportSource,
};
pub use library::VoiceLibrary;

use super::{
    instrumental::AudioLibrary,
    notify::Notify,
    plan::{OwnerId, Planner},
    render_cache::CacheStats,
    rendered::{PhonemeSnapshot, VisualTimeline},
    settings::VoiceSelection,
};
use crate::platform::web::{
    audio,
    client::Pending,
    protocol::{ComputeDiagnostics, Export, ExportMetrics, Inventory},
};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};
use crate::{tr};

pub enum VoiceState {
    Unloaded,
    Loading,
    Loaded(Box<Inventory>),
    Failed(String),
}

pub struct Runtime {
    pub library: VoiceLibrary,
    pub context_import: ContextImport,
    pub planner: Planner,
    pub stats: CacheStats,
    pub progress: Option<String>,
    pub error: Option<String>,
    pub failures: Vec<(OwnerId, String)>,
    pub export_result: Option<Result<Vec<PathBuf>, String>>,
    pub instrumentals: AudioLibrary,
    pub recording_error: Option<String>,
    repaint: Notify,
    voices: BTreeMap<VoiceSelection, VoiceState>,
    voice_jobs: BTreeMap<VoiceSelection, Pending<Inventory>>,
    visuals: BTreeMap<OwnerId, VisualTimeline>,
    phonemes: BTreeMap<OwnerId, PhonemeSnapshot>,
    preview: preview::State,
    transport: transport::State,
    diagnostics: Option<ComputeDiagnostics>,
    export: Option<Pending<Export>>,
    pub export_metrics: Option<ExportMetrics>,
}

impl Runtime {
    pub fn new(repaint: Notify) -> Self {
        Self {
            library: VoiceLibrary::new(repaint.clone()),
            context_import: ContextImport::default(),
            planner: Planner::default(),
            stats: CacheStats::default(),
            progress: None,
            error: None,
            failures: Vec::new(),
            export_result: None,
            instrumentals: AudioLibrary::default(),
            recording_error: None,
            repaint,
            voices: BTreeMap::new(),
            voice_jobs: BTreeMap::new(),
            visuals: BTreeMap::new(),
            phonemes: BTreeMap::new(),
            preview: preview::State::default(),
            transport: transport::State::default(),
            diagnostics: None,
            export: None,
            export_metrics: None,
        }
    }

    pub fn poll(&mut self) {
        self.instrumentals.poll();
        self.poll_library();
        self.poll_inventories();
        self.poll_preview();
        if let Some(result) = self.export.as_ref().and_then(Pending::poll) {
            self.export_result = Some(result.map(|export| {
                self.export_metrics = Some(export.metrics);
                self.diagnostics = Some(export.diagnostics);
                export.paths
            }));
            self.export = None;
        }
    }

    pub fn voice_state(&self, key: &VoiceSelection) -> &VoiceState {
        self.voices.get(key).unwrap_or(&VoiceState::Unloaded)
    }

    pub fn inventory(&self, key: &VoiceSelection) -> Option<&Inventory> {
        match self.voice_state(key) {
            VoiceState::Loaded(info) => Some(info),
            _ => None,
        }
    }

    pub fn voice_error(&self, key: &VoiceSelection) -> Option<&str> {
        match self.voice_state(key) {
            VoiceState::Failed(error) => Some(error),
            VoiceState::Loaded(info) if info.needs_context => Some(tr!("该声库需要便携模型上下文")),
            _ => None,
        }
    }

    pub fn load_elapsed(&self, key: &VoiceSelection) -> Option<Duration> {
        self.inventory(key)
            .and_then(|info| Duration::try_from_secs_f64(info.elapsed_ms / 1000.0).ok())
    }

    pub fn reset_project(&mut self) {
        self.invalidate_preview();
        self.clear_inventories();
        self.preview = preview::State::default();
        self.transport = transport::State::default();
        self.visuals.clear();
        self.phonemes.clear();
        self.planner.reset();
        self.instrumentals = AudioLibrary::default();
        self.recording_error = None;
        if let Some(task) = self.export.take() {
            task.cancel();
        }
        self.export_result = None;
        self.export_metrics = None;
        audio::audio_transport(false, 0.0);
        audio::audio_loop(0.0, 0.0);
    }

    pub fn visuals(&self, owner: OwnerId) -> Option<&VisualTimeline> {
        self.visuals.get(&owner)
    }
    pub fn phonemes(&self, owner: OwnerId) -> Option<&PhonemeSnapshot> {
        self.phonemes.get(&owner)
    }
    pub fn audio_duration(&self) -> Option<f64> {
        self.preview.duration
    }
    pub fn is_exporting(&self) -> bool {
        self.export.is_some()
    }
    pub fn playback_error(&self) -> Option<String> {
        audio::audio_error()
    }
    pub fn recording_failures(&self) -> impl Iterator<Item = &str> {
        self.recording_error.as_deref().into_iter()
    }

    pub fn status(&self) -> &'static str {
        if self.library.is_scanning() {
            tr!("正在扫描浏览器声库")
        } else if self.stats.running > 0 || self.stats.queued > 0 {
            tr!("正在合成歌声")
        } else if self.preview.is_pending() {
            tr!("正在准备预览")
        } else if !self.voice_jobs.is_empty() {
            tr!("正在加载声库")
        } else if self.error.is_some() {
            tr!("歌声合成失败 · 查看渲染面板")
        } else if self.preview.duration.is_some() {
            tr!("音频已就绪")
        } else {
            tr!("等待音符或伴奏")
        }
    }
}

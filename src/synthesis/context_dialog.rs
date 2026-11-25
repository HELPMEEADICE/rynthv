#[cfg(not(target_arch = "wasm32"))]
use super::runtime::context::ContextEvent;
use crate::synthesis::{
    settings::VoiceSelection,
    voice::session_identity::{self, OnlineContextSource},
};
use std::path::PathBuf;
use zeroize::Zeroize;

#[derive(Default)]
pub struct ContextImport {
    pub dialog: Option<ContextImportDialog>,
    pub feedback: Option<Result<String, String>>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) task: Option<std::sync::mpsc::Receiver<ContextEvent>>,
}

pub struct ContextImportDialog {
    pub selection: VoiceSelection,
    pub voice_name: String,
    pub model_version: u8,
    pub source: ContextImportSource,
    pub session_path: Option<PathBuf>,
    pub file_path: Option<PathBuf>,
    pub manual_identity: String,
    pub sources: Vec<OnlineContextSource>,
    pub selected_process: Option<u32>,
    pub phase: ContextImportPhase,
    pub error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ContextImportSource {
    SessionFile,
    File,
    Manual,
    Online,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ContextImportPhase {
    Discovering,
    Ready,
    Importing,
}

impl Drop for ContextImportDialog {
    fn drop(&mut self) {
        self.manual_identity.zeroize();
    }
}

impl ContextImportDialog {
    pub fn can_import(&self) -> bool {
        self.phase == ContextImportPhase::Ready
            && match self.source {
                ContextImportSource::SessionFile => {
                    session_identity::supports_session_import() && self.session_path.is_some()
                }
                ContextImportSource::File => self.file_path.is_some(),
                ContextImportSource::Manual => !self.manual_identity.trim().is_empty(),
                ContextImportSource::Online => self
                    .selected_process
                    .is_some_and(|id| self.sources.iter().any(|source| source.process_id == id)),
            }
    }
}

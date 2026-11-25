mod task;

pub use super::super::context_dialog::{
    ContextImport, ContextImportDialog, ContextImportPhase, ContextImportSource,
};
use super::Runtime;
use crate::synthesis::{
    settings::VoiceSelection,
    voice::session_identity::{self, OnlineContextSource},
};
use std::path::PathBuf;

pub(in crate::synthesis) enum ContextEvent {
    Sources(Result<Vec<OnlineContextSource>, String>),
    Imported(Result<PathBuf, String>),
}

impl Runtime {
    pub fn can_import_context(&self, selection: &VoiceSelection) -> bool {
        !self.library.is_scanning()
            && self.library.settings_error().is_none()
            && !self
                .context_import
                .dialog
                .as_ref()
                .is_some_and(|dialog| dialog.phase == ContextImportPhase::Importing)
            && self.library.entry(selection).is_some_and(|entry| {
                entry.availability.is_loadable()
                    && !entry.display_name.trim().is_empty()
                    && entry.family.is_some_and(|family| {
                        family.requires_identity()
                            && entry.model_version == Some(family.model_version())
                    })
            })
    }

    pub fn needs_context_import(&self, selection: &VoiceSelection) -> bool {
        self.library
            .entry(selection)
            .and_then(|entry| entry.family)
            .is_some_and(|family| family.requires_identity())
            && self
                .voices
                .get(selection)
                .is_some_and(|voice| voice.context_required)
    }

    /// Keep the failing singer as the import target even if the track changes.
    pub fn context_import_targets(&self) -> Vec<(VoiceSelection, String)> {
        self.voices
            .iter()
            .filter_map(|(selection, voice)| {
                if !voice.context_required || !self.needs_context_import(selection) {
                    return None;
                }
                let name = self
                    .inventory(selection)?
                    .configuration
                    .as_ref()?
                    .name
                    .clone();
                Some((selection.clone(), name))
            })
            .collect()
    }

    /// Opening the dialog never reads another application's session or processes.
    pub fn open_context_import(&mut self, selection: VoiceSelection) {
        if !self.can_import_context(&selection) {
            return;
        }
        let Some(entry) = self.library.entry(&selection) else {
            return;
        };
        let Some(model_version) = entry.model_version else {
            return;
        };
        self.context_import = ContextImport {
            dialog: Some(ContextImportDialog {
                voice_name: entry.display_name.clone(),
                model_version,
                selection,
                source: if session_identity::supports_session_import() {
                    ContextImportSource::SessionFile
                } else {
                    ContextImportSource::File
                },
                session_path: None,
                file_path: None,
                manual_identity: String::new(),
                sources: Vec::new(),
                selected_process: None,
                phase: ContextImportPhase::Ready,
                error: None,
            }),
            ..ContextImport::default()
        };
    }

    pub fn set_context_import_source(&mut self, source: ContextImportSource) {
        let Some(dialog) = &mut self.context_import.dialog else {
            return;
        };
        if dialog.phase == ContextImportPhase::Importing
            || source == dialog.source
            || (source == ContextImportSource::SessionFile
                && !session_identity::supports_session_import())
            || (source == ContextImportSource::Online
                && !session_identity::supports_online_import())
        {
            return;
        }
        // Dropping the receiver detaches the previous discovery, so its result
        // can never overwrite the fields of a newly selected source.
        self.context_import.task = None;
        dialog.source = source;
        dialog.phase = ContextImportPhase::Ready;
        self.clear_context_import_error();
        if source == ContextImportSource::Online {
            self.refresh_context_sources();
        }
    }

    pub fn clear_context_import_error(&mut self) {
        if let Some(dialog) = &mut self.context_import.dialog {
            dialog.error = None;
        }
        self.context_import.feedback = None;
    }

    pub fn refresh_context_sources(&mut self) {
        let Some(dialog) = &mut self.context_import.dialog else {
            return;
        };
        if matches!(
            dialog.phase,
            ContextImportPhase::Importing | ContextImportPhase::Discovering
        ) || self.context_import.task.is_some()
            || dialog.source != ContextImportSource::Online
            || !session_identity::supports_online_import()
        {
            return;
        }
        dialog.sources.clear();
        dialog.selected_process = None;
        dialog.error = None;
        dialog.phase = ContextImportPhase::Discovering;
        self.context_import.feedback = None;
        self.start_context_task("model-context-sources", || {
            ContextEvent::Sources(
                session_identity::online_context_sources().map_err(|error| error.to_string()),
            )
        });
    }

    pub fn cancel_context_import(&mut self) {
        if self
            .context_import
            .dialog
            .as_ref()
            .is_some_and(|dialog| dialog.phase != ContextImportPhase::Importing)
        {
            self.context_import.dialog = None;
            self.context_import.task = None;
        }
    }
}

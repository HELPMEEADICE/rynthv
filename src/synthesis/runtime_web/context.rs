use super::{ContextImportDialog, ContextImportPhase, ContextImportSource, Runtime};
use crate::{platform::web::protocol::Request, synthesis::settings::VoiceSelection};

impl Runtime {
    pub fn can_import_context(&self, selection: &VoiceSelection) -> bool {
        !self.library.resource_busy()
            && self.library.entry(selection).is_some_and(|entry| {
                entry.availability.is_loadable()
                    && entry
                        .family
                        .is_some_and(|family| family.requires_identity())
            })
    }

    pub fn needs_context_import(&self, selection: &VoiceSelection) -> bool {
        self.inventory(selection)
            .is_some_and(|inventory| inventory.needs_context)
    }

    pub fn context_import_targets(&self) -> Vec<(VoiceSelection, String)> {
        self.voices
            .keys()
            .filter(|key| self.needs_context_import(key))
            .filter_map(|key| Some((key.clone(), self.library.entry(key)?.display_name.clone())))
            .collect()
    }

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
        self.context_import.dialog = Some(ContextImportDialog {
            selection,
            voice_name: entry.display_name.clone(),
            model_version,
            source: ContextImportSource::File,
            session_path: None,
            file_path: None,
            manual_identity: String::new(),
            sources: Vec::new(),
            selected_process: None,
            phase: ContextImportPhase::Ready,
            error: None,
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
        }
    }

    pub fn clear_context_import_error(&mut self) {
        if let Some(dialog) = &mut self.context_import.dialog {
            dialog.error = None;
        }
        self.context_import.feedback = None;
    }

    pub fn set_context_import_source(&mut self, source: ContextImportSource) {
        if let Some(dialog) = &mut self.context_import.dialog
            && dialog.phase == ContextImportPhase::Ready
            && matches!(
                source,
                ContextImportSource::File | ContextImportSource::Manual
            )
        {
            dialog.source = source;
            self.clear_context_import_error();
        }
    }

    pub fn import_context(&mut self) {
        let Some(dialog) = self
            .context_import
            .dialog
            .as_ref()
            .filter(|dialog| dialog.can_import())
        else {
            return;
        };
        let request = Request::Context {
            name: dialog.voice_name.clone(),
            version: dialog.model_version,
            file: (dialog.source == ContextImportSource::File)
                .then(|| dialog.file_path.clone())
                .flatten(),
            identity: (dialog.source == ContextImportSource::Manual)
                .then(|| dialog.manual_identity.clone()),
        };
        if self.library_request(request)
            && let Some(dialog) = &mut self.context_import.dialog
        {
            dialog.phase = ContextImportPhase::Importing;
            dialog.error = None;
        }
    }
}

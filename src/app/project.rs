use super::App;
#[cfg(not(target_arch = "wasm32"))]
use crate::storage;
use crate::{editor::Editor, model::*, tr, ui};
#[cfg(not(target_arch = "wasm32"))]
use eframe::egui::Context;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

impl App {
    #[cfg(target_os = "android")]
    pub(super) fn list_local_projects(&mut self) {
        let result = (|| {
            let root = crate::platform::config_directory()?.join("projects");
            std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
            let mut files = Vec::new();
            for directory in root
                .read_dir()
                .map_err(|error| error.to_string())?
                .flatten()
            {
                if let Ok(entries) = directory.path().read_dir() {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_file()
                            && path
                                .extension()
                                .is_some_and(|ext| ext.eq_ignore_ascii_case("svp"))
                        {
                            let modified = entry.metadata().and_then(|meta| meta.modified()).ok();
                            files.push((modified, path));
                        }
                    }
                }
            }
            files.sort_by(|a, b| b.cmp(a));
            Ok(files.into_iter().map(|(_, path)| path).collect())
        })();
        match result {
            Ok(files) => self.local_projects = Some(files),
            Err(error) => self.error = Some(error),
        }
    }

    #[cfg(target_os = "android")]
    pub(super) fn local_project_dialog(&mut self, ctx: &Context) {
        use eframe::egui::{Button, Id, Modal, ScrollArea, vec2};
        let Some(files) = &self.local_projects else {
            return;
        };
        let mut selected = None;
        let mut close = false;
        let modal = Modal::new(Id::new("local-projects")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 30.0).clamp(220.0, 440.0));
            ui.heading(tr!("本机工程"));
            if files.is_empty() {
                ui.weak(tr!("保存的手机工程会显示在这里。"));
            }
            ScrollArea::vertical()
                .max_height(ctx.content_rect().height() * 0.55)
                .show(ui, |ui| {
                    for (index, file) in files.iter().enumerate() {
                        let name = file.file_stem().unwrap_or_default().to_string_lossy();
                        if ui
                            .add_sized(vec2(ui.available_width(), 44.0), Button::new(name))
                            .clicked()
                        {
                            selected = Some(index);
                        }
                    }
                });
            close = ui.button(tr!("关闭")).clicked();
        });
        if let Some(index) = selected {
            let path = files[index].clone();
            self.local_projects = None;
            self.open_path(path);
        } else if close || modal.should_close() {
            self.local_projects = None;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_path(&mut self, path: PathBuf) {
        ui::phonemes::phoneme_deactivate(self);
        ui::piano::cancel_audition(self);
        self.midi_import = None;
        match storage::load(&path) {
            Ok(storage::LoadedProject {
                project,
                imported,
                revision,
            }) => {
                self.install_project(project);
                self.path = (!imported).then_some(path);
                self.project_revision = (!imported).then_some(revision);
                self.status = if imported {
                    tr!("已导入 Synthesizer V Studio 1 工程，请另存为新文件").into()
                } else {
                    String::new()
                };
                #[cfg(target_os = "android")]
                {
                    self.recovered_dirty = imported;
                }
            }
            Err(e) => self.error = Some(e),
        }
    }

    pub(super) fn install_project(&mut self, project: Project) {
        self.editor = Editor::new(project);
        self.synthesis.reset_project();
        #[cfg(target_arch = "wasm32")]
        {
            self.browser.recovered_dirty = false;
        }
        #[cfg(target_os = "android")]
        {
            self.recovery.invalidate();
            self.recovered_dirty = false;
        }
        self.gesture = None;
        self.piano_anchor = ui::piano::AnchorSelection::default();
        self.parameter_lane = ui::lanes::ParameterSelection::default();
        self.phoneme_editing = ui::phonemes::PhonemeEditing::default();
        self.group_gesture = None;
        self.audio_drag = None;
        self.mixer_drag = None;
        self.audio_import = None;
        self.view_pan = ui::view_pan::ViewPan::default();
        self.rename = None;
        self.time_edit = None;
        self.bpm_edit = None;
        self.lyric = None;
        self.phoneme_draft = ui::panels::PhonemeDraft::default();
        self.fill_lyrics = None;
        let reference = self.editor.active_ref();
        let first = reference.and_then(|r| {
            self.editor.notes().iter().find_map(|n| {
                r.note_range(n)
                    .map(|(start, _)| (start, n.pitch + r.pitch_offset))
            })
        });
        self.view.piano.start = beats(first.map_or_else(
            || reference.map_or(0, |r| r.blick_absolute_begin),
            |(start, _)| start,
        ))
        .floor()
        .max(0.0);
        if let Some((_, pitch)) = first {
            self.view.top_pitch = (pitch + 9).clamp(12, 127) as f32;
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_project(&mut self, save_as: bool, _ctx: &Context) -> bool {
        ui::phonemes::phoneme_deactivate(self);
        ui::piano::cancel_audition(self);
        ui::tempo_input::finish(self);
        self.commit_lyric();
        self.editor.finish();
        #[cfg(not(target_os = "android"))]
        let path = if save_as || self.path.is_none() {
            let name = self
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or(tr!("未命名.svp"));
            rfd::FileDialog::new()
                .add_filter(tr!("Synthesizer V 工程"), &["svp"])
                .set_file_name(name)
                .save_file()
        } else {
            self.path.clone()
        };
        #[cfg(target_os = "android")]
        let path = match self.working_project_path() {
            Ok(path) => Some(path),
            Err(error) => {
                self.error = Some(error);
                return false;
            }
        };
        let Some(mut path) = path else {
            return false;
        };
        if path.extension().is_none() {
            path.set_extension("svp");
        }
        let result = (|| {
            let file = storage::ProjectFile::lock(&path)?;
            let revision = if self.path.as_ref() == Some(&path) {
                self.project_revision
            } else {
                storage::Revision::read(&path)?
            };
            file.save_if(&self.editor.project, revision)
        })();
        match result {
            Ok(revision) => {
                self.editor.mark_saved();
                self.path = Some(path.clone());
                self.project_revision = Some(revision);
                self.status = tr!("工程已保存").into();
                #[cfg(target_os = "android")]
                {
                    self.recovered_dirty = false;
                    if save_as {
                        self.pick(
                            _ctx,
                            super::picker::Picker::publish(path, "application/octet-stream"),
                            |app, _, _| app.status = tr!("工程副本已导出").into(),
                        );
                    }
                }
                true
            }
            Err(e) => {
                self.error = Some(e);
                false
            }
        }
    }

    #[cfg(target_os = "android")]
    fn working_project_path(&self) -> Result<PathBuf, String> {
        let root = crate::platform::config_directory()?.join("projects");
        if let Some(path) = self.path.as_ref().filter(|path| path.starts_with(&root)) {
            return Ok(path.clone());
        }
        let name = self
            .path
            .as_deref()
            .and_then(std::path::Path::file_name)
            .unwrap_or_else(|| std::ffi::OsStr::new(tr!("未命名.svp")));
        let directory = root.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        Ok(directory.join(name))
    }

    #[cfg(target_os = "android")]
    pub(crate) fn share_project(&mut self, ctx: &Context) {
        if self.save_project(false, ctx)
            && let Some(path) = &self.path
            && let Err(error) = crate::platform::android::share(path, "application/octet-stream")
        {
            self.error = Some(error);
        }
    }
}

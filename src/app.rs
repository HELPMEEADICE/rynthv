mod instrumental;
mod midi;
#[cfg(target_os = "android")]
mod mobile;
pub(crate) mod picker;
mod project;
#[cfg(target_os = "android")]
mod recovery;
mod voice;
#[cfg(target_arch = "wasm32")]
pub(crate) mod web;
pub use midi::{MidiImport, MidiImportMode, MidiImportPhase};

use crate::{
    editor::Editor,
    model::*,
    synthesis, tr,
    ui::{self, state::ViewState},
};
#[cfg(not(target_arch = "wasm32"))]
use crate::{storage, trf};
use eframe::egui::*;
use std::{path::PathBuf, time::Duration};
use web_time::Instant;

#[derive(Clone, Copy)]
pub enum FileAction {
    New,
    Open,
    #[cfg(any(target_os = "android", target_arch = "wasm32"))]
    OpenLocal,
    #[cfg(target_arch = "wasm32")]
    OpenDropped,
    ImportMidi,
    #[cfg(not(target_os = "android"))]
    Close,
}
pub struct App {
    pub editor: Editor,
    pub synthesis: synthesis::runtime::Runtime,
    pub graphics_api: String,
    pub view: ViewState,
    pub preferences: ui::settings::Preferences,
    pub path: Option<PathBuf>,
    #[cfg(not(target_arch = "wasm32"))]
    project_revision: Option<storage::Revision>,
    pub error: Option<String>,
    pub settings: bool,
    pub voice_manager: bool,
    pub about: bool,
    pub piano_tool: ui::piano::Tool,
    pub piano_anchor: ui::piano::AnchorSelection,
    pub parameter_lane: ui::lanes::ParameterSelection,
    pub phoneme_editing: ui::phonemes::PhonemeEditing,
    pub show_pitch: bool,
    pub show_waveform: bool,
    pub show_retakes: bool,
    pub show_time_tracks: bool,
    pub piano_collapsed: bool,
    pub piano_audition: ui::piano::KeyboardAudition,
    pub view_pan: ui::view_pan::ViewPan,
    pub gesture: Option<ui::piano::Gesture>,
    pub group_gesture: Option<ui::group_edit::GroupGesture>,
    pub audio_drag: Option<ui::instrumental::AudioDrag>,
    pub mixer_drag: Option<(EntityId, &'static str, u64)>,
    pub audio_import: Option<instrumental::AudioImport>,
    pub midi_import: Option<MidiImport>,
    pub rename: Option<ui::rename::Rename>,
    pub time_edit: Option<ui::time_edit::TimeEdit>,
    pub bpm_edit: Option<ui::tempo_input::BpmEdit>,
    pub lyric: Option<ui::piano::LyricEdit>,
    pub phoneme_draft: ui::panels::PhonemeDraft,
    pub fill_lyrics: Option<ui::fill_lyrics::FillLyrics>,
    pub pending: Option<FileAction>,
    #[cfg(target_os = "android")]
    picker: Option<picker::Completion>,
    #[cfg(target_os = "android")]
    recovery: recovery::Recovery,
    #[cfg(target_os = "android")]
    recovered_dirty: bool,
    #[cfg(target_os = "android")]
    audio_wanted: bool,
    #[cfg(target_os = "android")]
    local_projects: Option<Vec<PathBuf>>,
    #[cfg(target_arch = "wasm32")]
    browser: web::State,
    #[cfg(not(target_os = "android"))]
    pub allow_close: bool,
    pub status: String,
    #[cfg(not(target_arch = "wasm32"))]
    settings_path: Option<PathBuf>,
    last_layout: String,
    last_preferences: String,
    #[cfg(not(target_os = "android"))]
    last_title: String,
    last_persist: Instant,
}
impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        path: Option<PathBuf>,
        #[cfg(target_arch = "wasm32")] start: web::Startup,
    ) -> Self {
        ui::theme::install(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let mut error = None;
        #[cfg(not(target_arch = "wasm32"))]
        let settings_path = match crate::platform::config_directory() {
            Ok(directory) => Some(directory.join("workspace.json")),
            Err(message) => {
                error = Some(message);
                None
            }
        };
        let mut view = ViewState::default();
        let mut last_layout = String::new();
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = settings_path.as_ref().filter(|path| path.exists()) {
            match std::fs::read(path)
                .map_err(|e| e.to_string())
                .and_then(|b| serde_json::from_slice::<ViewState>(&b).map_err(|e| e.to_string()))
            {
                Ok(mut v) if v.valid() => {
                    for l in &mut v.lanes {
                        l.id = new_id();
                    }
                    last_layout = serde_json::to_string_pretty(&v).unwrap_or_default();
                    view = v;
                }
                Ok(_) => error = Some(tr!("保存的工作区尺寸或结构无效，已重置布局").into()),
                Err(e) => error = Some(trf!("无法读取工作区：{}", e)),
            }
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(mut saved) = start.view.filter(ViewState::valid) {
            for lane in &mut saved.lanes {
                lane.id = new_id();
            }
            last_layout = serde_json::to_string_pretty(&saved).unwrap_or_default();
            view = saved;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let preferences = match settings_path
            .as_ref()
            .map(|path| ui::settings::Preferences::load(&path.with_file_name("preferences.json")))
            .transpose()
        {
            Ok(preferences) => preferences.unwrap_or_default(),
            Err(message) => {
                error = Some(message);
                ui::settings::Preferences::default()
            }
        };
        #[cfg(target_arch = "wasm32")]
        let preferences = start
            .preferences
            .filter(ui::settings::Preferences::valid)
            .unwrap_or_default();
        #[cfg(not(target_arch = "wasm32"))]
        if let Err(message) = ui::theme::fonts(&cc.egui_ctx, preferences.locale) {
            error = error.or(Some(message));
        }
        #[cfg(target_arch = "wasm32")]
        if let Err(message) = ui::theme::web_fonts(&cc.egui_ctx, start.font) {
            error = error.or(Some(message));
        }
        preferences.apply(&cc.egui_ctx);
        cc.egui_ctx
            .options_mut(|options| options.zoom_with_keyboard = false);
        let last_preferences =
            serde_json::to_string_pretty(&preferences).expect("valid interface preferences");
        let mut app = Self {
            editor: Editor::new(Project::default()),
            synthesis: synthesis::runtime::Runtime::new({
                let ctx = cc.egui_ctx.clone();
                synthesis::notify::Notify::new(move || ctx.request_repaint())
            }),
            graphics_api: cc
                .wgpu_render_state
                .as_ref()
                .map(|state| format!("{:?}", state.adapter.get_info().backend))
                .unwrap_or_else(|| tr!("未初始化").into()),
            view,
            preferences,
            path: None,
            #[cfg(not(target_arch = "wasm32"))]
            project_revision: None,
            error,
            settings: false,
            voice_manager: false,
            about: false,
            piano_tool: ui::piano::Tool::NoteSelect,
            piano_anchor: ui::piano::AnchorSelection::default(),
            parameter_lane: ui::lanes::ParameterSelection::default(),
            phoneme_editing: ui::phonemes::PhonemeEditing::default(),
            show_pitch: true,
            show_waveform: true,
            show_retakes: true,
            show_time_tracks: true,
            piano_collapsed: false,
            piano_audition: ui::piano::KeyboardAudition::default(),
            view_pan: ui::view_pan::ViewPan::default(),
            gesture: None,
            group_gesture: None,
            audio_drag: None,
            mixer_drag: None,
            audio_import: None,
            midi_import: None,
            rename: None,
            time_edit: None,
            bpm_edit: None,
            lyric: None,
            phoneme_draft: ui::panels::PhonemeDraft::default(),
            fill_lyrics: None,
            pending: None,
            #[cfg(target_os = "android")]
            picker: None,
            #[cfg(target_os = "android")]
            recovery: recovery::Recovery::default(),
            #[cfg(target_os = "android")]
            recovered_dirty: false,
            #[cfg(target_os = "android")]
            audio_wanted: false,
            #[cfg(target_os = "android")]
            local_projects: None,
            #[cfg(target_arch = "wasm32")]
            browser: web::State::default(),
            #[cfg(not(target_os = "android"))]
            allow_close: false,
            status: String::new(),
            #[cfg(not(target_arch = "wasm32"))]
            settings_path,
            last_layout,
            last_preferences,
            #[cfg(not(target_os = "android"))]
            last_title: String::new(),
            last_persist: Instant::now(),
        };
        if let Some(path) = path {
            app.open_path(path);
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(recovery) = start.recovery {
            app.restore_web_project(recovery);
        }
        #[cfg(target_os = "android")]
        match recovery::Recovery::init(&cc.egui_ctx) {
            Ok((recovery, snapshot)) => {
                app.recovery = recovery;
                if let Some(snapshot) = snapshot
                    && let Err(error) = app.restore_snapshot(snapshot)
                {
                    app.error = Some(error);
                }
            }
            Err(error) => app.error = Some(error),
        }
        app
    }

    fn has_unsaved_changes(&self) -> bool {
        #[cfg(target_os = "android")]
        if self.recovered_dirty {
            return true;
        }
        #[cfg(target_arch = "wasm32")]
        if self.browser.recovered_dirty {
            return true;
        }
        self.editor.dirty()
    }
    pub fn request_file(&mut self, action: FileAction, ctx: &Context) {
        ui::phonemes::phoneme_deactivate(self);
        ui::piano::cancel_audition(self);
        ui::tempo_input::finish(self);
        self.time_edit = None;
        self.commit_lyric();
        self.fill_lyrics = None;
        self.editor.finish();
        if self.has_unsaved_changes() {
            self.pending = Some(action);
        } else {
            self.perform_file(action, ctx);
        }
    }
    fn perform_file(&mut self, action: FileAction, ctx: &Context) {
        ui::phonemes::phoneme_deactivate(self);
        self.midi_import = None;
        self.gesture = None;
        self.piano_anchor = ui::piano::AnchorSelection::default();
        self.parameter_lane = ui::lanes::ParameterSelection::default();
        self.phoneme_editing = ui::phonemes::PhonemeEditing::default();
        self.group_gesture = None;
        self.audio_drag = None;
        self.mixer_drag = None;
        self.audio_import = None;
        self.synthesis.instrumentals.finish_import();
        self.view_pan.cancel(ctx);
        self.rename = None;
        self.time_edit = None;
        self.lyric = None;
        self.phoneme_draft = ui::panels::PhonemeDraft::default();
        self.fill_lyrics = None;
        self.bpm_edit = None;
        match action {
            FileAction::New => {
                self.install_project(Project::default());
                self.path = None;
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.project_revision = None;
                }
            }
            FileAction::Open => self.pick_one(
                ctx,
                picker::Picker::new(picker::Selection::File, tr!("打开工程"))
                    .filter(tr!("Synthesizer V 工程"), &["svp"]),
                |app, _, path| app.open_path(path),
            ),
            #[cfg(any(target_os = "android", target_arch = "wasm32"))]
            FileAction::OpenLocal => self.list_local_projects(),
            #[cfg(target_arch = "wasm32")]
            FileAction::OpenDropped => {
                if let Some(path) = self.browser.dropped_project.take() {
                    self.open_path(path);
                }
            }
            FileAction::ImportMidi => self.begin_midi_import(MidiImportMode::NewProject, ctx),
            #[cfg(not(target_os = "android"))]
            FileAction::Close => {
                self.allow_close = true;
                self.persist_layout();
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
    }
    pub fn edit_lyric(&mut self) {
        if let Some(note) = self
            .editor
            .notes()
            .iter()
            .find(|n| self.editor.selected.contains(&n.id))
        {
            self.lyric = Some(ui::piano::LyricEdit {
                id: note.id,
                text: note.lyrics.clone(),
                focus: true,
            });
        }
    }
    pub fn commit_lyric(&mut self) {
        if let Some(lyric) = self.lyric.take() {
            self.editor.set_lyric(lyric.id, lyric.text);
        }
        if self.phoneme_draft.dirty {
            let draft = std::mem::take(&mut self.phoneme_draft);
            let still_current = self
                .editor
                .project
                .group(&draft.group)
                .is_some_and(|group| {
                    draft
                        .selection
                        .iter()
                        .zip(&draft.source)
                        .all(|(id, source)| {
                            group
                                .notes
                                .iter()
                                .any(|n| n.id == *id && n.phonemes == *source)
                        })
                });
            if still_current && !self.editor.is_editing() {
                self.editor
                    .set_note_phonemes(&draft.group, &draft.selection, &draft.text);
            }
        }
    }
    #[cfg(not(target_os = "android"))]
    fn update_title(&mut self, _ctx: &Context) {
        let title = format!(
            "RynthV - {}{}",
            self.path
                .as_ref()
                .and_then(|p| p.file_stem())
                .and_then(|p| p.to_str())
                .unwrap_or(tr!("未命名")),
            if self.has_unsaved_changes() { " *" } else { "" }
        );
        if title != self.last_title {
            self.last_title = title.clone();
            #[cfg(target_arch = "wasm32")]
            web::set_title(&title);
            #[cfg(not(target_arch = "wasm32"))]
            _ctx.send_viewport_cmd(ViewportCommand::Title(title));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn persist_layout(&mut self) {
        self.last_persist = Instant::now();
        let Some(settings_path) = self.settings_path.as_ref() else {
            return;
        };
        if let Ok(json) = serde_json::to_string_pretty(&self.preferences)
            && json != self.last_preferences
        {
            let path = settings_path.with_file_name("preferences.json");
            let result = std::fs::create_dir_all(path.parent().unwrap())
                .map_err(|e| e.to_string())
                .and_then(|_| storage::atomic_write(&path, json.as_bytes()));
            match result {
                Ok(()) => self.last_preferences = json,
                Err(error) => self.error = Some(error),
            }
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.view)
            && json != self.last_layout
        {
            let result = std::fs::create_dir_all(settings_path.parent().unwrap())
                .map_err(|e| e.to_string())
                .and_then(|_| storage::atomic_write(settings_path, json.as_bytes()));
            match result {
                Ok(()) => self.last_layout = json,
                Err(e) => self.error = Some(e),
            }
        }
    }
    fn dialogs(&mut self, ctx: &Context) {
        #[cfg(any(target_os = "android", target_arch = "wasm32"))]
        self.local_project_dialog(ctx);
        if let Some(action) = self.pending {
            Modal::new(Id::new("unsaved")).show(ctx, |ui| {
                ui.heading(tr!("保存工程更改？"));
                ui.label(tr!("当前工程包含尚未保存的编辑。"));
                ui.horizontal(|ui| {
                    if ui.button(tr!("保存")).clicked() && self.save_project(false, ctx) {
                        self.pending = None;
                        self.perform_file(action, ctx);
                    }
                    if ui.button(tr!("不保存")).clicked() {
                        self.pending = None;
                        self.perform_file(action, ctx);
                    }
                    if ui.button(tr!("取消")).clicked() {
                        self.pending = None;
                    }
                });
            });
        }
        if let Some(error) = self.error.clone() {
            Modal::new(Id::new("error")).show(ctx, |ui| {
                ui.set_max_width(480.0);
                ui.heading(tr!("操作未完成"));
                ui.label(error);
                if ui.button(tr!("确定")).clicked() {
                    self.error = None;
                }
            });
        }
        if self.about {
            Modal::new(Id::new("about")).show(ctx, |ui| {
                ui.heading(format!("RynthV {}", env!("CARGO_PKG_VERSION")));
                ui.label(tr!("Rust + egui 歌声合成引擎"));
                ui.label(tr!("多音轨编辑、歌词与音高调整、MIDI 导入和 WAV 导出。"));
                ui.label(tr!("支持 Windows、Linux、Android 和 Web。"));
                if ui.button(tr!("关闭")).clicked() {
                    self.about = false;
                }
            });
        }
        if self.settings {
            ui::settings::show(self, ctx);
        }
        ui::voice_library::dialogs(self, ctx);
        #[cfg(target_os = "android")]
        if self.picker.is_some() {
            Modal::new(Id::new("android-file-operation")).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tr!("正在处理文件…"));
                });
            });
        }
    }
}
impl eframe::App for App {
    #[cfg(target_os = "android")]
    fn raw_input_hook(&mut self, ctx: &Context, raw: &mut RawInput) {
        crate::platform::android::input::read(ctx, raw);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        #[cfg(target_arch = "wasm32")]
        self.poll_browser(&ctx);
        #[cfg(target_os = "android")]
        {
            self.handle_mobile_events(&ctx);
            self.poll_picker(&ctx);
            if !crate::platform::android::lifecycle::foreground() {
                return;
            }
        }
        ui::piano::begin_keyboard_frame(self, &ctx);
        self.synthesis.poll();
        self.poll_audio_import();
        self.poll_midi_import();
        #[cfg(not(target_os = "android"))]
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.request_file(FileAction::Close, &ctx);
        }
        let audio_seconds = self.synthesis.playback_clock(self.editor.transport.playing);
        let looped = self.editor.transport.update(
            &self.editor.project.time,
            self.editor.project_end().max(32 * QUARTER),
            audio_seconds,
        );
        let transport_position = self.editor.transport.position;
        if self.editor.transport.playing {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        if self.pending.is_none()
            && self.error.is_none()
            && !self.settings
            && !self.voice_manager
            && self.synthesis.context_import.dialog.is_none()
            && !self.about
            && self.rename.is_none()
            && self.time_edit.is_none()
            && self.bpm_edit.is_none()
            && self.fill_lyrics.is_none()
            && self.midi_import.is_none()
            && !Popup::is_any_open(&ctx)
        {
            ui::chrome::shortcuts(self, &ctx);
        }
        #[cfg(not(target_os = "android"))]
        self.update_title(&ctx);
        ui.painter()
            .rect_filled(ui.max_rect(), 0, ui::theme::background_color());
        #[cfg(target_os = "android")]
        ui::mobile::show(self, ui, ctx.content_rect().intersect(ui.max_rect()));
        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
        ui::chrome::desktop(self, ui);
        #[cfg(target_arch = "wasm32")]
        if ui::mobile::compact(&ctx) {
            ui::mobile::show(self, ui, ctx.content_rect().intersect(ui.max_rect()));
        } else {
            ui::chrome::desktop(self, ui);
        }
        ui::instrumental::update_drag(self, &ctx);
        ui::group_edit::end_frame(self, &ctx);
        self.dialogs(&ctx);
        ui::rename::show(self, &ctx);
        if self.pending.is_none() && self.error.is_none() {
            ui::time_edit::show(self, &ctx);
            ui::fill_lyrics::show(self, &ctx);
            ui::midi_import::show(self, &ctx);
        }
        self.view_pan.end_frame(&ctx);
        ui::piano::end_keyboard_frame(self, &ctx);
        #[cfg(target_os = "android")]
        self.sync_android_audio();
        ui::piano::end_gesture_frame(self, &ctx);
        ui::piano::end_pitch_frame(self, &ctx);
        ui::piano::end_anchor_frame(self, &ctx);
        ui::lanes::end_parameter_frame(self, &ctx);
        ui::phonemes::end_phoneme_frame(self, &ctx);
        ui::panels::end_performance_frame(self, &ctx);
        ui::voice_library::end_parameter_frame(self, &ctx);
        ui::track_header::end_frame(self, &ctx);
        if !self.editor.is_editing() {
            self.synthesis
                .sync_project(&self.editor.project, self.editor.revision);
        }
        if self.synthesis.recording_error.is_some() {
            self.editor.transport.playing = false;
        } else if self.editor.transport.playing {
            self.editor.transport.constrain_to_loop();
        }
        self.synthesis.set_loop_range(
            self.editor
                .transport
                .loop_seconds(&self.editor.project.time),
        );
        self.synthesis.sync_transport(
            self.editor.transport.playing,
            self.editor
                .project
                .time
                .seconds_at(self.editor.transport.position)
                .max(0.0),
            looped || self.editor.transport.position != transport_position,
        );
        if self.last_persist.elapsed() > Duration::from_secs(3) {
            self.persist_layout();
        }
        #[cfg(target_os = "android")]
        {
            self.update_recovery(&ctx);
            crate::platform::android::input::publish(&ctx);
        }
    }
    fn on_exit(&mut self) {
        ui::piano::cancel_audition(self);
        self.persist_layout();
    }
}

use super::App;
use crate::{platform::android::lifecycle, ui};
use eframe::egui::{Context, Popup};

impl App {
    pub(super) fn handle_mobile_events(&mut self, ctx: &Context) {
        let events = lifecycle::take_events();
        if events & lifecycle::INTERRUPTED != 0 {
            if let Some(seconds) = self.synthesis.playback_clock(self.editor.transport.playing) {
                self.editor.transport.position = self.editor.project.time.position_at(seconds);
            }
            self.editor.transport.playing = false;
            self.synthesis.sync_transport(
                false,
                self.editor
                    .project
                    .time
                    .seconds_at(self.editor.transport.position)
                    .max(0.0),
                true,
            );
            ui::piano::cancel_audition(self);
            self.sync_android_audio();
        }
        if events & lifecycle::PAUSED != 0 {
            self.persist_layout();
            self.update_recovery(ctx);
        }
        if events & (lifecycle::AUDIO_ROUTE | lifecycle::PAUSED | lifecycle::RESUMED) != 0 {
            if let Some(seconds) = self.synthesis.playback_clock(self.editor.transport.playing) {
                self.editor.transport.position = self.editor.project.time.position_at(seconds);
            }
            self.synthesis.reset_android_audio(
                self.editor
                    .project
                    .time
                    .seconds_at(self.editor.transport.position)
                    .max(0.0),
                lifecycle::foreground(),
            );
        }
        if events & lifecycle::TRIM_MEMORY != 0 {
            self.synthesis.release_mobile_caches();
        }
        if events & lifecycle::BACK != 0 {
            self.back(ctx);
        }
    }

    pub(super) fn sync_android_audio(&mut self) {
        let wanted = lifecycle::foreground()
            && (self.editor.transport.playing || self.piano_audition.is_sounding());
        if wanted == self.audio_wanted {
            return;
        }
        self.audio_wanted = wanted;
        match lifecycle::request_audio(wanted) {
            Ok(granted) if !wanted || granted => return,
            Ok(_) => self.status = "系统音频焦点暂不可用".into(),
            Err(error) => self.error = Some(error),
        }
        self.audio_wanted = false;
        self.editor.transport.playing = false;
        ui::piano::cancel_audition(self);
    }

    fn back(&mut self, ctx: &Context) {
        if Popup::is_any_open(ctx) {
            Popup::close_all(ctx);
        } else if self.error.take().is_some() || self.pending.take().is_some() {
        } else if self.synthesis.context_import.dialog.is_some() {
            self.synthesis.cancel_context_import();
        } else if self.settings {
            self.settings = false;
        } else if self.voice_manager {
            self.voice_manager = false;
        } else if self.local_projects.is_some() {
            self.local_projects = None;
        } else if self.about {
            self.about = false;
        } else if self.midi_import.is_some() {
            self.midi_import = None;
        } else if self.fill_lyrics.is_some() || self.rename.is_some() || self.time_edit.is_some() {
            self.fill_lyrics = None;
            self.rename = None;
            self.time_edit = None;
        } else if ctx.egui_wants_keyboard_input() {
            ctx.memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
        } else if !ui::mobile::back_to_editor(ctx) {
            self.persist_layout();
            self.update_recovery(ctx);
            if let Err(error) = crate::platform::android::move_to_background() {
                self.error = Some(error);
            }
        }
    }
}

use super::App;
use crate::model::{AudioClip, EntityId};
use eframe::egui::Context;
use std::path::PathBuf;

pub struct AudioImport {
    pub track: EntityId,
    path: PathBuf,
    onset: f64,
    placement: Option<AudioClip>,
}

impl App {
    pub fn import_audio(&mut self, target: Option<EntityId>, ctx: &Context) {
        self.select_audio(target, false, ctx);
    }

    pub fn relink_audio(&mut self, target: EntityId, ctx: &Context) {
        self.select_audio(Some(target), true, ctx);
    }

    fn select_audio(&mut self, target: Option<EntityId>, relink: bool, ctx: &Context) {
        crate::ui::phonemes::phoneme_deactivate(self);
        let Some(path) = rfd::FileDialog::new()
            .set_title(if relink {
                "重新链接音频"
            } else {
                "导入伴奏"
            })
            .add_filter(
                "音频文件",
                &[
                    "wav", "mp3", "flac", "ogg", "aif", "aiff", "aac", "m4a", "mp4",
                ],
            )
            .pick_file()
        else {
            return;
        };
        self.commit_lyric();
        self.editor.finish();
        let track = target.unwrap_or_else(|| self.editor.add_instrumental_track());
        self.begin_audio_import(track, path, relink, ctx);
    }

    pub fn reload_audio(&mut self, track: EntityId, ctx: &Context) {
        if let Some(path) = self
            .editor
            .project
            .tracks
            .iter()
            .find(|t| t.id == track)
            .and_then(|t| t.audio_clip())
            .map(|c| c.path.clone())
        {
            self.synthesis.instrumentals.retry(&path, ctx);
        }
    }

    fn begin_audio_import(&mut self, track: EntityId, path: PathBuf, relink: bool, ctx: &Context) {
        let clip = self
            .editor
            .project
            .tracks
            .iter()
            .find(|t| t.id == track)
            .and_then(|t| t.audio_clip());
        let onset = clip.map_or(0.0, |c| c.onset_seconds);
        let placement = clip.filter(|_| relink).cloned();
        self.synthesis.instrumentals.begin_import(path.clone(), ctx);
        self.audio_import = Some(AudioImport {
            track,
            path,
            onset,
            placement,
        });
        self.status = "正在导入伴奏…".into();
    }

    pub(super) fn poll_audio_import(&mut self) {
        let Some(import) = &self.audio_import else {
            return;
        };
        if !self
            .editor
            .project
            .tracks
            .iter()
            .any(|t| t.id == import.track && t.is_instrumental())
        {
            self.audio_import = None;
            self.synthesis.instrumentals.finish_import();
            self.status.clear();
            return;
        }
        if let Some(error) = self.synthesis.instrumentals.error(&import.path) {
            self.error = Some(error.to_owned());
            self.audio_import = None;
            self.synthesis.instrumentals.finish_import();
            self.status = "伴奏导入失败".into();
            return;
        }
        let Some(asset) = self.synthesis.instrumentals.asset(&import.path) else {
            return;
        };
        // Never combine completion with an unrelated in-progress gesture.
        if self.editor.is_editing() {
            return;
        }
        let duration = asset.audio.duration_seconds();
        let import = self.audio_import.take().unwrap();
        self.synthesis.instrumentals.finish_import();
        let clip = if let Some(mut clip) = import.placement {
            if duration <= clip.source_offset_seconds {
                self.error = Some("文件的时长短于片段的源起点，无法重新链接".into());
                self.status = "重新链接音频失败".into();
                return;
            }
            clip.path = import.path;
            clip
        } else {
            AudioClip {
                path: import.path,
                onset_seconds: import.onset,
                duration_seconds: duration,
                source_offset_seconds: 0.0,
            }
        };
        self.editor.set_audio_clip(import.track, Some(clip));
        self.status = "伴奏已加载，可拖动片段调整位置".into();
    }
}

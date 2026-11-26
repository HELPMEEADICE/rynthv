use super::Editor;
use crate::model::*;
use crate::{tr, trf};

impl Editor {
    pub fn add_instrumental_track(&mut self) -> EntityId {
        self.begin(tr!("创建伴奏轨"));
        let mut track = Track::new(self.project.tracks.len());
        track.disp_color = self.project.next_track_color();
        track.name = trf!("伴奏 {}", (self.project.tracks.len() + 1));
        track.instrumental = Some(InstrumentalTrack::default());
        track
            .main_ref
            .extra
            .insert("isInstrumental".into(), true.into());
        let id = track.id;
        self.project.tracks.push(track);
        self.finish();
        id
    }

    pub fn set_audio_clip(&mut self, id: EntityId, clip: Option<AudioClip>) {
        if clip.as_ref().is_some_and(|c| !c.valid()) {
            return;
        }
        self.begin(if clip.is_some() {
            tr!("导入伴奏")
        } else {
            tr!("移除伴奏")
        });
        if let Some(track) = self.project.tracks.iter_mut().find(|t| t.id == id)
            && let Some(instrumental) = &mut track.instrumental
        {
            instrumental.clip = clip;
        }
        self.finish();
    }

    /// Caller owns the drag transaction, so a complete drag is one undo step.
    pub fn move_audio_clip(&mut self, id: EntityId, seconds: f64) {
        if !seconds.is_finite() {
            return;
        }
        if let Some(clip) = self
            .project
            .tracks
            .iter_mut()
            .find(|t| t.id == id)
            .and_then(|t| t.instrumental.as_mut())
            .and_then(|t| t.clip.as_mut())
        {
            clip.onset_seconds = seconds.max(0.0);
        }
    }
}

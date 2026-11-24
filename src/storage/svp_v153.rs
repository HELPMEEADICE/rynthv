use crate::model::{AudioClip, GroupRef, InstrumentalTrack, Project, SVP_VERSION, TimeAxis, beats};
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;

mod synthesis;

pub(super) const VERSION: i32 = 153;

pub(super) fn import(project: &mut Project) -> Result<(), String> {
    if project.uuid.is_empty() {
        project.uuid = uuid::Uuid::new_v4().to_string();
    }
    for group in project
        .library
        .iter_mut()
        .chain(project.tracks.iter_mut().map(|track| &mut track.main_group))
    {
        for note in &mut group.notes {
            if note.uuid.is_empty() {
                note.renew_identity();
            }
            synthesis::import_note(note)
                .map_err(|error| format!("SVP 音符组 {} · {}：{error}", group.name, note.lyrics))?;
        }
    }
    for track in &mut project.tracks {
        if track.main_ref.extra.remove("isInstrumental") == Some(Value::Bool(true)) {
            let clip = import_clip(&project.time, &mut track.main_ref)
                .map_err(|error| format!("SVP 伴奏轨 {}：{error}", track.name))?;
            track.instrumental = Some(InstrumentalTrack { clip });
            track.main_ref.blick_absolute_begin = 0;
            track.main_ref.blick_absolute_end = -1;
            track.main_ref.blick_offset = 0;
        }
        for reference in std::iter::once(&mut track.main_ref).chain(&mut track.groups) {
            if reference.uuid.is_empty() {
                reference.renew_identity();
            }
            synthesis::import_reference(reference);
        }
    }
    project.version = SVP_VERSION;
    Ok(())
}

#[derive(Deserialize)]
struct AudioFile {
    filename: PathBuf,
    duration: f64,
}

fn import_clip(time: &TimeAxis, reference: &mut GroupRef) -> Result<Option<AudioClip>, String> {
    if reference.blick_absolute_end != -1
        && reference.blick_absolute_end < reference.blick_absolute_begin
    {
        return Err("片段可见范围无效".into());
    }
    let Some(audio) = reference.extra.remove("audio") else {
        return Ok(None);
    };
    let audio: AudioFile = serde_json::from_value(audio).map_err(|error| error.to_string())?;
    if audio.filename.as_os_str().is_empty() && audio.duration == 0.0 {
        return Ok(None);
    }
    if audio.filename.as_os_str().is_empty() || !audio.duration.is_finite() || audio.duration <= 0.0
    {
        return Err("音频文件路径或时长无效".into());
    }
    let seconds_at = |position| {
        if position < 0 {
            time.start_time_seconds + beats(position) * 60.0 / time.tempo[0].bpm
        } else {
            time.seconds_at(position)
        }
    };
    let source_start = seconds_at(reference.blick_offset);
    let begin = seconds_at(reference.blick_absolute_begin)
        .max(source_start)
        .max(0.0);
    let mut end = source_start + audio.duration;
    if reference.blick_absolute_end != -1 {
        end = end.min(seconds_at(reference.blick_absolute_end));
    }
    if !begin.is_finite() || !end.is_finite() {
        return Err("伴奏时间超出有效范围".into());
    }
    Ok((end > begin).then_some(AudioClip {
        path: audio.filename,
        onset_seconds: begin,
        duration_seconds: end - begin,
        source_offset_seconds: begin - source_start,
    }))
}

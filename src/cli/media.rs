mod analysis;
mod render;

use super::{project::Selection, target, *};
use crate::{
    editor::Editor,
    midi,
    model::{AudioClip, Project},
    synthesis::instrumental,
};

impl Workstation {
    pub(super) fn edit_media(
        &mut self,
        call: Call<'_>,
        editor: &mut Editor,
        selection: &Selection,
    ) -> Result<Value> {
        match (call.family, call.action) {
            ("tempo", action) => {
                let at = target::position(&editor.project.time, "beat", call.required("beat")?)?;
                if action == "set" {
                    editor.set_tempo(at, target::number(call.required("bpm")?)?)?;
                } else {
                    if at == 0 {
                        return Err(invalid("The first tempo marker is required"));
                    }
                    editor.delete_tempo(at);
                }
                tempo(&editor.project)
            }
            ("timesig", action) => {
                let bar = call
                    .number::<i32>("bar")?
                    .filter(|&bar| bar > 0)
                    .ok_or_else(|| invalid("Bar numbers begin at 1"))?
                    - 1;
                if action == "set" {
                    let (n, d) = target::signature(call.required("value")?)?;
                    editor.set_meter(bar, n, d)?;
                } else {
                    if bar == 0 {
                        return Err(invalid("The first meter is required"));
                    }
                    editor.delete_meter(bar);
                }
                meter(&editor.project)
            }
            ("midi", "import") => {
                let input = call.required("input")?;
                let midi = if input == "-" {
                    midi::parse(&self.read(input)?)
                } else {
                    midi::load(Path::new(input))
                }
                .map_err(|error| Failure::new("MIDI_INVALID", error))?;
                let indices: Vec<usize> = if let Some(tracks) = call.get("tracks") {
                    tracks
                        .split(',')
                        .map(|item| {
                            item.trim()
                                .parse()
                                .map_err(|_| invalid(format!("Invalid MIDI track index: {item}")))
                        })
                        .collect::<Result<_>>()?
                } else {
                    midi.tracks
                        .iter()
                        .enumerate()
                        .filter_map(|(index, track)| (!track.notes.is_empty()).then_some(index))
                        .collect()
                };
                let summary = editor.import_midi_tracks(&midi, &indices)?;
                if call.flag("time-map") {
                    editor.project.time = midi.time;
                }
                Ok(
                    json!({"tracks":summary.tracks,"notes":summary.notes,"split_tracks":summary.split_tracks,
                    "time_map_imported":call.flag("time-map"),"warnings":midi.warnings}),
                )
            }
            ("audio", "import") => {
                let path = std::path::absolute(call.path("input")?)
                    .map_err(|error| Failure::new("AUDIO_RESOURCE", error.to_string()))?;
                let audio = instrumental::read_audio(&path, &self.cancellation)
                    .map_err(|error| Failure::new("AUDIO_RESOURCE", error))?;
                let clip = AudioClip {
                    path,
                    onset_seconds: call
                        .get("offset")
                        .map(target::number)
                        .transpose()?
                        .unwrap_or(0.0),
                    duration_seconds: audio.duration_seconds(),
                    source_offset_seconds: 0.0,
                };
                if !clip.valid() {
                    return Err(invalid("Invalid accompaniment placement"));
                }
                let id = if let Some(name) = call.get("track") {
                    let track = target::track(&editor.project, Some(name))?;
                    if !track.is_instrumental() {
                        return Err(invalid("Choose an accompaniment track"));
                    }
                    track.id
                } else {
                    editor.add_instrumental_track()
                };
                if let Some(name) = call.get("name") {
                    editor.rename_track(id, name.to_owned());
                } else if call.get("track").is_none() {
                    editor.rename_track(id, clip.name().to_owned());
                }
                editor.set_audio_clip(id, Some(clip));
                Ok(tracks::track_json(
                    editor
                        .project
                        .tracks
                        .iter()
                        .find(|track| track.id == id)
                        .expect("audio track"),
                ))
            }
            ("audio", "set") => {
                let track = target::track(&editor.project, selection.track(call))?;
                let id = track.id;
                let mut clip = track
                    .audio_clip()
                    .ok_or_else(|| invalid("Track has no accompaniment clip"))?
                    .clone();
                let fields = [
                    ("offset", &mut clip.onset_seconds),
                    ("source-offset", &mut clip.source_offset_seconds),
                    ("duration", &mut clip.duration_seconds),
                ];
                for (key, field) in fields {
                    if let Some(value) = call.get(key) {
                        *field = target::number(value)?;
                    }
                }
                if !clip.valid() {
                    return Err(invalid("Invalid accompaniment time range"));
                }
                editor.set_audio_clip(id, Some(clip.clone()));
                Ok(json!({"clip":clip}))
            }
            _ => Err(invalid("Unknown media edit")),
        }
    }

    pub(super) fn media_query(&self, call: Call<'_>) -> Result<Value> {
        let path = call.path("source")?;
        if call.family == "midi" {
            let midi = midi::load(path).map_err(|error| Failure::new("MIDI_INVALID", error))?;
            return Ok(
                json!({"source":path,"tracks":midi.tracks,"time":midi.time,"warnings":midi.warnings,
                "selection_indices":"Zero-based index in the tracks array"}),
            );
        }
        let audio = instrumental::read_audio(path, &self.cancellation)
            .map_err(|error| Failure::new("AUDIO_RESOURCE", error))?;
        let estimate = if call.flag("f0") {
            Some(analysis::fundamental(&audio, &self.cancellation)?)
        } else {
            None
        };
        Ok(json!({"source":path,"measurements":analysis::measure(&audio),"estimated_f0":estimate}))
    }
    pub(super) fn query_media(
        &self,
        call: Call<'_>,
        project: &Project,
        selection: &Selection,
    ) -> Result<Value> {
        match call.family {
            "tempo" => tempo(project),
            "timesig" => meter(project),
            "midi" => {
                let path = call.path("output")?;
                separate_from_source(call.path("project")?, path)?;
                let overwrite = call.flag("overwrite");
                check_output(path, overwrite)?;
                let track = selection
                    .track(call)
                    .map(|track| target::track(project, Some(track)).map(|track| track.id))
                    .transpose()?;
                let count =
                    midi::save_project(path, project, track, overwrite, call.flag("dry-run"))?;
                Ok(
                    json!({"output":path,"track_count":count,"dry_run":call.flag("dry-run"),
                    "warnings":["MIDI approximates note timing and omits voicebanks, phoneme overrides, tuning curves, performance, effects and audio"]}),
                )
            }
            _ => Err(invalid("Unknown media query")),
        }
    }
}
fn tempo(project: &Project) -> Result<Value> {
    Ok(
        json!({"tempos":project.time.tempo.iter().map(|event| json!({"blick":event.position,
        "beat":crate::model::beats(event.position),"bpm":event.bpm,"seconds":project.time.seconds_at(event.position)})).collect::<Vec<_>>()}),
    )
}
fn meter(project: &Project) -> Result<Value> {
    Ok(
        json!({"meters":project.time.meter.iter().map(|event| json!({"bar":i64::from(event.index)+1,"numerator":event.numerator,
        "denominator":event.denominator,"blick":project.time.bar_position(event.index)})).collect::<Vec<_>>()}),
    )
}

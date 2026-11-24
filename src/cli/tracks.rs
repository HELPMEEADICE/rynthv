use super::{project::Selection, target, *};
use crate::{
    editor::Editor,
    model::{GroupRef, Project, QUARTER, Track},
};
use std::collections::BTreeMap;

pub(super) fn track_json(track: &Track) -> Value {
    json!({
        "id": track.main_ref.uuid, "name":track.name, "index":track.disp_order + 1,
        "kind":if track.is_instrumental() { "instrumental" } else { "vocal" },
        "group_count":track.groups.len() + usize::from(!track.is_instrumental()),
        "main_group_id":track.main_group.uuid, "voice":track.main_ref.database,
        "mixer":track.mixer,"audio":track.audio_clip()
    })
}
fn reference_json(project: &Project, track: &Track, reference: &GroupRef) -> Value {
    let group = project
        .group(&reference.group_id)
        .expect("validated reference");
    json!({"id": reference.uuid, "group_id":group.uuid,"track_id":track.main_ref.uuid,"name":group.name,
        "main": reference.id == track.main_ref.id, "note_count":group.notes.len(),
        "start_blick":reference.blick_absolute_begin,"end_blick":(reference.blick_absolute_end >= 0).then_some(reference.blick_absolute_end),
        "offset_blick":reference.blick_offset,"pitch_offset_semitones":reference.pitch_offset,
        "mute":reference.mute,"voice":reference.database,
        "reference_count":project.tracks.iter().flat_map(Track::references).filter(|other| other.group_id == reference.group_id).count()})
}
pub(super) fn query(call: Call<'_>, project: &Project, selection: &Selection) -> Result<Value> {
    if call.family == "track" {
        return Ok(json!({"tracks":project.tracks.iter().map(track_json).collect::<Vec<_>>()}));
    }
    let track = target::track(project, selection.track(call))?;
    Ok(
        json!({"groups":if track.is_instrumental() { Vec::new() } else {
        track.references().map(|reference| reference_json(project, track, reference)).collect() }}),
    )
}
pub(super) fn edit(call: Call<'_>, editor: &mut Editor, selection: &Selection) -> Result<Value> {
    if call.family == "track" {
        return edit_track(call, editor, selection);
    }
    let track = target::scope_track(
        &editor.project,
        selection.track(call),
        selection.group(call),
    )?;
    if track.is_instrumental() {
        return Err(invalid("Choose a vocal track"));
    }
    let track_id = track.id;
    let unit = call.get("unit").unwrap_or("beat");
    let at = call
        .get("at")
        .map(|value| target::position(&editor.project.time, unit, value))
        .transpose()?;
    if call.action == "add" {
        let start = at.unwrap_or(0);
        let duration = call
            .get("duration")
            .map(|value| target::duration(&editor.project.time, unit, start, value))
            .transpose()?
            .unwrap_or(32 * QUARTER);
        if start.checked_add(duration).is_none() {
            return Err(invalid("Group range overflow"));
        }
        editor.begin("Create group");
        let id = editor.insert_group(track_id, start, duration);
        editor.finish();
        editor.rename_group(id, call.required("name")?.to_owned());
        return instance_result(&editor.project, id);
    }
    let reference = target::reference(track, selection.group(call))?.clone();
    let destination = if let Some(track) = call.get("to-track") {
        target::track(&editor.project, Some(track))?.id
    } else {
        track_id
    };
    let delta = at.unwrap_or(reference.blick_absolute_begin) - reference.blick_absolute_begin;
    let id = match call.action {
        "delete" => {
            if track.main_ref.id == reference.id {
                return Err(invalid(
                    "The main group cannot be deleted independently of its track",
                ));
            }
            editor.delete_group(reference.id);
            return Ok(json!({"removed":reference.uuid}));
        }
        "set" => {
            if let Some(name) = call.get("name") {
                target::edit_reference(&editor.project, &reference, call.flag("shared"))?;
                editor.rename_group(reference.id, name.to_owned());
            }
            let transpose = call.number::<i32>("semitones")?;
            if transpose.is_some_and(|value| !(-127..=127).contains(&value)) {
                return Err(invalid(
                    "Group transpose must be between -127 and 127 semitones",
                ));
            }
            if let Some(value) = transpose
                && editor
                    .project
                    .group(&reference.group_id)
                    .expect("selected group")
                    .notes
                    .iter()
                    .any(|note| !(0..=127).contains(&(note.pitch + value)))
            {
                return Err(invalid(
                    "Group transpose would put a note outside MIDI pitch range",
                ));
            }
            editor.begin("Set group instance");
            let selected = editor
                .project
                .tracks
                .iter_mut()
                .flat_map(|track| std::iter::once(&mut track.main_ref).chain(&mut track.groups))
                .find(|other| other.id == reference.id)
                .expect("selected instance");
            if let Some(transpose) = transpose {
                selected.pitch_offset = transpose;
            }
            if let Some(mute) = call.number::<bool>("mute")? {
                selected.mute = mute;
            }
            editor.finish();
            reference.id
        }
        "copy" | "link" => {
            editor.copy_instance(&reference, destination, call.action == "link", delta)?
        }
        "independent" => {
            editor.make_instance_independent(reference.id)?;
            reference.id
        }
        "move" => {
            editor.move_instance(reference.id, destination, delta)?;
            reference.id
        }
        "split" => editor.split_instance(
            reference.id,
            at.ok_or_else(|| invalid("Missing split position"))?,
        )?,
        "merge" => {
            let next = target::reference(track, call.get("with"))?.id;
            editor.merge_instances(reference.id, next)?;
            reference.id
        }
        _ => return Err(invalid("Unknown group command")),
    };
    instance_result(&editor.project, id)
}
fn instance_result(project: &Project, id: u64) -> Result<Value> {
    project
        .tracks
        .iter()
        .find_map(|track| {
            track
                .references()
                .find(|reference| reference.id == id)
                .map(|reference| reference_json(project, track, reference))
        })
        .ok_or_else(|| invalid("Group instance was not created"))
}
fn edit_track(call: Call<'_>, editor: &mut Editor, selection: &Selection) -> Result<Value> {
    let get_id = || target::track(&editor.project, selection.track(call)).map(|track| track.id);
    let id = match call.action {
        "add" if call.flag("instrumental") => editor.add_instrumental_track(),
        "add" => {
            editor.add_track();
            editor.track
        }
        "copy" => editor
            .copy_track(get_id()?)
            .ok_or_else(|| invalid("Track missing"))?,
        "delete" => {
            let id = get_id()?;
            if editor.project.tracks.len() == 1 {
                return Err(invalid("Cannot delete the last track"));
            }
            let track = editor
                .project
                .tracks
                .iter()
                .find(|track| track.id == id)
                .expect("selected track");
            let uuid = track.main_ref.uuid.clone();
            editor.delete_track(id);
            return Ok(json!({"removed":uuid}));
        }
        "set" => {
            let id = get_id()?;
            let mut properties = BTreeMap::new();
            if let Some(gain) = call.get("volume") {
                let gain = target::number(gain)?;
                if !(10.0_f64.powf(gain / 20.0) as f32).is_finite() {
                    return Err(invalid("Volume gain overflows"));
                }
                properties.insert("gainDecibel", json!(gain));
            }
            if let Some(pan) = call.get("pan") {
                let pan = target::number(pan)?;
                if !(-1.0..=1.0).contains(&pan) {
                    return Err(invalid("Pan must be between -1 and 1"));
                }
                properties.insert("pan", json!(pan));
            }
            for key in ["mute", "solo"] {
                if let Some(value) = call.number::<bool>(key)? {
                    properties.insert(key, json!(value));
                }
            }
            if let Some(order) = call.number::<usize>("index")? {
                if !(1..=editor.project.tracks.len()).contains(&order) {
                    return Err(invalid("Track order is out of range"));
                }
                let index = editor
                    .project
                    .tracks
                    .iter()
                    .position(|track| track.id == id)
                    .expect("selected track");
                editor.reorder_track(id, order as isize - 1 - index as isize);
            }
            editor.begin("Set track mixer");
            editor
                .project
                .tracks
                .iter_mut()
                .find(|track| track.id == id)
                .expect("selected track")
                .mixer
                .extend(
                    properties
                        .into_iter()
                        .map(|(key, value)| (key.to_owned(), value)),
                );
            editor.finish();
            id
        }
        _ => return Err(invalid("Unknown track command")),
    };
    if let Some(name) = call.get("name") {
        editor.rename_track(id, name.to_owned());
    }
    Ok(track_json(
        editor
            .project
            .tracks
            .iter()
            .find(|track| track.id == id)
            .expect("selected track"),
    ))
}

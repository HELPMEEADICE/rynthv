use super::{project::Selection, target, *};
use crate::{
    editor::Editor,
    model::{GroupRef, Language, Note, Project, QUARTER},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub(super) struct Located {
    pub reference: GroupRef,
    pub note: Note,
    pub track_id: String,
}
impl Located {
    pub fn json(&self, project: &Project) -> Value {
        let mut value = target::note_json(&self.note, &self.reference, &project.time);
        value["track_id"] = json!(self.track_id);
        value
    }
    fn onset(&self) -> i64 {
        self.note.onset + self.reference.blick_offset
    }
    fn pitch(&self) -> i32 {
        self.note.pitch.saturating_add(self.reference.pitch_offset)
    }
}
pub(super) fn selected(
    call: Call<'_>,
    project: &Project,
    selection: &Selection,
) -> Result<Vec<Located>> {
    let track_filter = selection.track(call);
    let group_filter = selection.group(call);
    let track_id = if track_filter.is_some() {
        Some(target::track(project, track_filter)?.id)
    } else {
        None
    };
    let unit = call.get("unit").unwrap_or("beat");
    let bound = |key| {
        call.get(key)
            .map(|value| target::position(&project.time, unit, value))
            .transpose()
    };
    let from = bound("from")?.unwrap_or(i64::MIN);
    let to = bound("to")?.unwrap_or(i64::MAX);
    let lo = call
        .get("min-pitch")
        .map(target::pitch)
        .transpose()?
        .unwrap_or(0);
    let hi = call
        .get("max-pitch")
        .map(target::pitch)
        .transpose()?
        .unwrap_or(127);
    if from >= to || lo > hi {
        return Err(invalid("Invalid selection range"));
    }
    let mut result = Vec::new();
    let mut group_found = group_filter.is_none();
    for track in project
        .tracks
        .iter()
        .filter(|track| !track.is_instrumental() && track_id.is_none_or(|id| id == track.id))
    {
        for reference in track.references().filter(|reference| {
            group_filter.is_none_or(|id| reference.uuid.eq_ignore_ascii_case(id))
        }) {
            group_found = true;
            let group = project.group(&reference.group_id).expect("validated group");
            for note in &group.notes {
                let Some((start, end)) = reference.note_range(note) else {
                    continue;
                };
                let pitch = note.pitch.saturating_add(reference.pitch_offset);
                if end <= from
                    || start >= to
                    || pitch < lo
                    || pitch > hi
                    || call
                        .get("note")
                        .is_some_and(|id| !note.uuid.eq_ignore_ascii_case(id))
                {
                    continue;
                }
                result.push(Located {
                    reference: reference.clone(),
                    note: note.clone(),
                    track_id: track.main_ref.uuid.clone(),
                });
            }
        }
    }
    if !group_found {
        return Err(Failure::new(
            "GROUP_NOT_FOUND",
            "Requested group reference was not found",
        ));
    }
    if result.is_empty()
        && let Some(id) = call.get("note")
    {
        return Err(Failure::new(
            "NOTE_NOT_FOUND",
            "No visible note matches the UUID and filters",
        )
        .details(json!({"note_id":id})));
    }
    result.sort_by_key(|item| {
        (
            item.onset(),
            item.pitch(),
            item.track_id.clone(),
            item.note.uuid.clone(),
        )
    });
    Ok(result)
}
pub(super) fn editable(
    call: Call<'_>,
    project: &Project,
    selection: &Selection,
) -> Result<Vec<Located>> {
    let notes = selected(call, project, selection)?;
    if notes.is_empty() {
        return Err(Failure::new(
            "NOTE_NOT_FOUND",
            "The selection contains no notes",
        ));
    }
    let mut ids = BTreeSet::new();
    for item in &notes {
        target::edit_reference(project, &item.reference, call.flag("shared"))?;
        if !ids.insert(&item.note.uuid) {
            return Err(Failure::new(
                "AMBIGUOUS_NOTE",
                "Select a single instance with --group before editing linked notes",
            )
            .details(json!({"note_id":item.note.uuid})));
        }
    }
    Ok(notes)
}
pub(super) fn one(items: &[Located]) -> Result<&Located> {
    match items {
        [item] => Ok(item),
        _ => Err(invalid(
            "Select exactly one note using --note and, for a shared definition, --group",
        )),
    }
}
pub(super) fn note_mut<'a>(project: &'a mut Project, located: &Located) -> &'a mut Note {
    project
        .group_mut(&located.reference.group_id)
        .expect("selected group")
        .notes
        .iter_mut()
        .find(|note| note.id == located.note.id)
        .expect("selected note")
}
pub(super) fn query(call: Call<'_>, project: &Project, selection: &Selection) -> Result<Value> {
    let notes = selected(call, project, selection)?;
    if call.action == "overlaps" {
        let mut previous: BTreeMap<&str, (&Located, i64)> = BTreeMap::new();
        let mut overlaps = Vec::new();
        for item in notes.iter().filter(|item| !item.note.muted()) {
            let (start, end) = item.reference.note_range(&item.note).expect("visible note");
            let key = item.reference.uuid.as_str();
            if let Some((prev, previous_end)) = previous.get(key)
                && start < *previous_end
            {
                overlaps.push(json!({"group_ref_id":key,"first_note_id":prev.note.uuid,"second_note_id":item.note.uuid,
                    "start_blick":start,"end_blick":end.min(*previous_end)}));
            }
            if previous
                .get(key)
                .is_none_or(|(_, previous_end)| end > *previous_end)
            {
                previous.insert(key, (item, end));
            }
        }
        return Ok(json!({"overlaps":overlaps}));
    }
    Ok(json!({"notes":notes.iter().map(|item| item.json(project)).collect::<Vec<_>>()}))
}
#[derive(serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct NoteEdit {
    at: Option<Value>,
    duration: Option<Value>,
    pitch: Option<Value>,
    lyric: Option<String>,
    phonemes: Option<String>,
    language: Option<String>,
    mute: Option<bool>,
}
impl NoteEdit {
    fn parse(call: Call<'_>) -> Result<Self> {
        let get = |key| call.get(key).map(str::to_owned);
        Ok(Self {
            at: get("at").map(Value::String),
            duration: get("duration").map(Value::String),
            pitch: get("pitch").map(Value::String),
            lyric: get("lyric"),
            phonemes: get("phonemes"),
            language: get("language"),
            mute: call.number("mute")?,
        })
    }
    fn offset(&self, project: &Project, unit: &str) -> Result<Option<i64>> {
        self.at
            .as_ref()
            .map(|value| target::position(&project.time, unit, &scalar(value)?))
            .transpose()
    }
    fn apply(
        self,
        note: &mut Note,
        reference: &GroupRef,
        time: &crate::model::TimeAxis,
        unit: &str,
    ) -> Result<()> {
        if let Some(value) = self.at {
            let position = target::position(time, unit, &scalar(&value)?)?;
            set_position(note, reference, position)?;
        }
        if let Some(value) = self.duration {
            note.duration = target::duration(
                time,
                unit,
                note.onset
                    .checked_add(reference.blick_offset)
                    .ok_or_else(|| invalid("Project note position overflow"))?,
                &scalar(&value)?,
            )?;
        }
        if let Some(value) = self.pitch {
            note.pitch = target::pitch(&scalar(&value)?)?
                .checked_sub(reference.pitch_offset)
                .filter(|pitch| (0..=127).contains(pitch))
                .ok_or_else(|| invalid("Pitch is out of range after the instance's transpose"))?;
        }
        if let Some(lyric) = self.lyric {
            note.replace_lyric(&lyric);
        }
        if let Some(value) = self.phonemes {
            note.replace_phonemes(&value);
        }
        if let Some(language) = self.language {
            let language = Language::parse(&language)
                .ok_or_else(|| invalid(format!("Unknown language {language}")))?;
            note.set_language_override(Some(language));
        }
        if let Some(mute) = self.mute {
            note.attributes.insert("muted".into(), json!(mute));
        }
        Ok(())
    }
}

impl Workstation {
    pub(super) fn edit_notes(
        &mut self,
        call: Call<'_>,
        editor: &mut Editor,
        selection: &Selection,
    ) -> Result<Value> {
        if matches!(call.action, "add" | "import") {
            let track = target::scope_track(
                &editor.project,
                selection.track(call),
                selection.group(call),
            )?;
            if track.is_instrumental() {
                return Err(invalid("Choose a vocal track"));
            }
            let reference = target::reference(track, selection.group(call))?.clone();
            let track_id = track.main_ref.uuid.clone();
            target::edit_reference(&editor.project, &reference, call.flag("shared"))?;
            let edits = if call.action == "add" {
                vec![NoteEdit::parse(call)?]
            } else {
                self.read_json::<Vec<NoteEdit>>(call.required("input")?)?
            };
            let unit = call.get("unit").unwrap_or("beat");
            let mut notes = Vec::new();
            for edit in edits {
                if edit.at.is_none() || edit.duration.is_none() || edit.pitch.is_none() {
                    return Err(invalid("Each new note needs at, duration and pitch"));
                }
                let mut note = Note::new(0, 1, 60);
                edit.apply(&mut note, &reference, &editor.project.time, unit)?;
                notes.push(note);
            }
            editor
                .project
                .group_mut(&reference.group_id)
                .expect("selected group")
                .notes
                .extend(notes.iter().cloned());
            let values: Vec<_> = notes
                .into_iter()
                .map(|note| {
                    Located {
                        reference: reference.clone(),
                        note,
                        track_id: track_id.clone(),
                    }
                    .json(&editor.project)
                })
                .collect();
            return Ok(json!({"notes":values}));
        }
        let items = editable(call, &editor.project, selection)?;
        if call.family == "lyrics" {
            return self.fill_lyrics(call, &mut editor.project, items);
        }
        let project = &mut editor.project;
        let unit = call.get("unit").unwrap_or("beat");
        let time = project.time.clone();
        let affected = items
            .iter()
            .map(|item| item.note.uuid.as_str())
            .collect::<Vec<_>>();
        let mut created = Vec::new();
        match call.action {
            "set" | "move" => {
                let edit = NoteEdit::parse(call)?;
                let item = one(&items)?;
                edit.apply(note_mut(project, item), &item.reference, &time, unit)?;
            }
            "delete" => remove(project, &items),
            "copy" => {
                let edit = NoteEdit::parse(call)?;
                let delta = edit
                    .offset(project, unit)?
                    .map(|at| {
                        at.checked_sub(items[0].onset())
                            .ok_or_else(|| invalid("Copy offset overflow"))
                    })
                    .transpose()?
                    .unwrap_or(0);
                for item in &items {
                    let mut note = item.note.clone();
                    note.renew_identity();
                    set_position(
                        &mut note,
                        &item.reference,
                        item.onset()
                            .checked_add(delta)
                            .ok_or_else(|| invalid("Copy position overflow"))?,
                    )?;
                    created.push(note.uuid.clone());
                    project
                        .group_mut(&item.reference.group_id)
                        .expect("selected group")
                        .notes
                        .push(note);
                }
            }
            "transpose" => {
                let delta = call
                    .number::<i32>("semitones")?
                    .ok_or_else(|| invalid("Missing semitone change"))?;
                for item in &items {
                    let pitch = item
                        .note
                        .pitch
                        .checked_add(delta)
                        .filter(|pitch| (0..=127).contains(pitch))
                        .filter(|pitch| {
                            pitch
                                .checked_add(item.reference.pitch_offset)
                                .is_some_and(|pitch| (0..=127).contains(&pitch))
                        })
                        .ok_or_else(|| {
                            invalid(format!(
                                "Transpose puts note {} outside MIDI pitch range",
                                item.note.uuid
                            ))
                        })?;
                    note_mut(project, item).pitch = pitch;
                }
            }
            "quantize" => {
                let (n, d) = target::signature(call.required("grid")?)?;
                if n == 0 || d == 0 {
                    return Err(invalid("Grid must be a positive whole-note fraction"));
                }
                let grid = QUARTER
                    .checked_mul(4 * i64::from(n))
                    .map(|value| value / i64::from(d))
                    .filter(|&value| value > 0)
                    .ok_or_else(|| invalid("Grid is outside Blick resolution"))?;
                let snap = |value: i64| {
                    let value = (i128::from(value) + i128::from(grid) / 2)
                        .div_euclid(i128::from(grid))
                        * i128::from(grid);
                    i64::try_from(value).map_err(|_| invalid("Grid position overflow"))
                };
                for item in &items {
                    let at = snap(item.onset())?;
                    let end = snap(item.onset() + item.note.duration)?;
                    let note = note_mut(project, item);
                    set_position(note, &item.reference, at)?;
                    note.duration = end
                        .checked_sub(at)
                        .ok_or_else(|| invalid("Quantized duration overflow"))?
                        .max(grid);
                }
            }
            "split" => {
                let item = one(&items)?;
                let split = target::position(&time, unit, call.required("at")?)?;
                let offset = split
                    .checked_sub(item.onset())
                    .filter(|&value| value > 0 && value < item.note.duration)
                    .ok_or_else(|| invalid("Split point must fall inside the note"))?;
                let mut right = item.note.clone();
                right.renew_identity();
                set_position(&mut right, &item.reference, split)?;
                right.replace_lyric("-");
                right.duration -= offset;
                note_mut(project, item).duration = offset;
                created.push(right.uuid.clone());
                project
                    .group_mut(&item.reference.group_id)
                    .expect("selected group")
                    .notes
                    .push(right);
            }
            "merge" => {
                let first = &items[0];
                let mut end = first.note.onset + first.note.duration;
                if items.len() < 2 {
                    return Err(invalid("Select two or more notes"));
                }
                for item in &items[1..] {
                    if item.reference.uuid != first.reference.uuid
                        || item.pitch() != first.pitch()
                        || item.note.onset > end
                    {
                        return Err(invalid(
                            "Merge requires adjacent or overlapping notes of the same pitch and instance",
                        ));
                    }
                    end = end.max(item.note.onset + item.note.duration);
                }
                note_mut(project, first).duration = end
                    .checked_sub(first.note.onset)
                    .ok_or_else(|| invalid("Merged duration overflow"))?;
                remove(project, &items[1..]);
            }
            _ => return Err(invalid("Unknown note edit")),
        }
        Ok(json!({"affected":affected,"created":created}))
    }

    fn fill_lyrics(
        &self,
        call: Call<'_>,
        project: &mut Project,
        items: Vec<Located>,
    ) -> Result<Value> {
        let text = if let Some(path) = call.get("input") {
            String::from_utf8(self.read(path)?).map_err(|_| invalid("Lyrics must use UTF-8"))?
        } else {
            call.required("text")?.to_owned()
        };
        let language = call.get("language").and_then(Language::parse);
        let mut lyrics = Vec::new();
        if call.flag("syllabify") || (language.is_some() && !call.flag("literal")) {
            let language = language.ok_or_else(|| invalid("--syllabify requires --language"))?;
            let frontend = resources::dictionary_frontend(language)?;
            for word in crate::editor::split_lyrics(&text, false) {
                let syllables = frontend.resolve(language, &word).map_err(|error| {
                    Failure::new("LYRIC_UNRESOLVED", error.to_string())
                        .details(json!({"word":word,"language":language}))
                })?;
                for syllable in syllables {
                    lyrics.push((word.clone(), Some(syllable.join(" "))));
                }
            }
        } else {
            lyrics.extend(
                crate::editor::split_lyrics(&text, false)
                    .into_iter()
                    .map(|word| (word, None)),
            );
        }
        if lyrics.len() != items.len() {
            return Err(Failure::new(
                "LYRIC_COUNT",
                "Supply one lyric or resolved syllable per selected note",
            )
            .details(json!({"notes":items.len(),"syllables":lyrics.len()})));
        }
        let mut mapped = Vec::new();
        for (item, (lyric, phones)) in items.iter().zip(lyrics) {
            let note = note_mut(project, item);
            note.replace_lyric(&lyric);
            if let Some(phones) = phones {
                note.replace_phonemes(&phones);
            }
            if let Some(language) = language {
                note.set_language_override(Some(language));
            }
            mapped.push(json!({"note_id":note.uuid,"group_ref_id":item.reference.uuid,"lyric":note.lyrics,"phonemes":note.phonemes}));
        }
        Ok(json!({"mapping":mapped}))
    }
}
fn scalar(value: &Value) -> Result<String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Number(value) => Ok(value.to_string()),
        _ => Err(invalid("Time and pitch values must be strings or numbers")),
    }
}
fn set_position(note: &mut Note, reference: &GroupRef, at: i64) -> Result<()> {
    if !reference.contains(at) {
        return Err(invalid("New onset is outside the group reference"));
    }
    note.onset = at
        .checked_sub(reference.blick_offset)
        .ok_or_else(|| invalid("Local note position overflow"))?;
    Ok(())
}
fn remove(project: &mut Project, notes: &[Located]) {
    let mut groups: BTreeMap<&str, BTreeSet<u64>> = BTreeMap::new();
    for item in notes {
        groups
            .entry(&item.reference.group_id)
            .or_default()
            .insert(item.note.id);
    }
    for (group, ids) in groups {
        project
            .group_mut(group)
            .expect("selected group")
            .notes
            .retain(|note| !ids.contains(&note.id));
    }
}

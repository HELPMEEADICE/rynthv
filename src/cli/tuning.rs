use super::{notes, project::Selection, resources, target, *};
use crate::{
    editor::Editor,
    model::{
        Automation, AutomationTarget, GroupRef, NotePerformance, Parameter, PhonemeAttributes,
        PitchControl, Project, RetakeComponent, TakeSet, Track, VoiceSettings,
    },
    synthesis::{
        plan::{OwnerId, Planner},
        score::{self, LyricFrontend},
        voice::VoiceFamily,
    },
};
use std::{collections::BTreeMap, sync::Arc};

pub(super) fn parameter_specs() -> Vec<Value> {
    Parameter::ALL.into_iter().map(|parameter| json!({"name":parameter.key(),"editor_range":parameter.range(),
        "default":parameter.default_value(),"unit":match parameter { Parameter::Pitch | Parameter::ToneShift => "cents",
            Parameter::Loudness => "decibels", _ => "normalized" }})).collect()
}
fn scope<'a>(
    call: Call<'a>,
    project: &'a Project,
    selection: &'a Selection,
) -> Result<(&'a Track, &'a GroupRef)> {
    let track = target::scope_track(project, selection.track(call), selection.group(call))?;
    if track.is_instrumental() {
        return Err(invalid("Choose a vocal track"));
    }
    Ok((track, target::reference(track, selection.group(call))?))
}
fn pitch_json(project: &Project, reference: &GroupRef) -> Value {
    let group = project.group(&reference.group_id).expect("validated group");
    json!({"time_unit":"local_blick","pitch_delta_unit":"cents","pitch_control_unit":"semitones",
        "pitch_delta":group.parameters.get(Parameter::Pitch.key()),"pitch_controls":group.pitch_controls,
        "system_pitch_delta":reference.system_pitch_delta,
        "notes":group.notes.iter().filter(|note| reference.note_range(note).is_some())
            .map(|note| target::note_json(note, reference, &project.time)).collect::<Vec<_>>()})
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PitchEdit {
    pitch_delta: Option<Automation>,
    pitch_controls: Option<Vec<PitchControl>>,
    system_pitch_delta: Option<Option<Automation>>,
}
impl Workstation {
    pub(super) fn query_tuning(
        &mut self,
        call: Call<'_>,
        project: &Project,
        selection: &Selection,
    ) -> Result<Value> {
        match (call.family, call.action) {
            ("pitch", action) => {
                let (_, reference) = scope(call, project, selection)?;
                let mut pitch = pitch_json(project, reference);
                if action == "export" {
                    // Export uses the import shape so it can be reapplied without conversion.
                    let path = call.path("output")?;
                    let data = json!({"pitch_delta":pitch["pitch_delta"],"pitch_controls":pitch["pitch_controls"],
                        "system_pitch_delta":pitch["system_pitch_delta"]});
                    json_file(path, &data, call)?;
                    pitch["output"] = json!(path);
                }
                Ok(pitch)
            }
            ("parameter", "list" | "voice") => {
                let (_, reference) = scope(call, project, selection)?;
                let group = project.group(&reference.group_id).expect("validated group");
                if call.action == "voice" {
                    return Ok(
                        json!({"group_ref_id":reference.uuid,"voice":reference.extra.get("voice")}),
                    );
                }
                Ok(
                    json!({"group_ref_id":reference.uuid,"time_unit":"local_blick","parameters":parameter_specs(),
                    "curves":group.parameters,"vocal_modes":group.vocal_modes,"vocal_mode_curve_range_percent":[-150,150]}),
                )
            }
            ("parameter", "note") => Ok(
                json!({"notes":notes::selected(call, project, selection)?.iter()
                .map(|item| json!({"note_id":item.note.uuid,"group_ref_id":item.reference.uuid,
                    "performance":NotePerformance::from_note(&item.note).ok(),"attributes":item.note.attributes,
                    "sv1_pitch":item.note.rynthv_sv1_pitch,"musical_type":item.note.musical_type,"accent":item.note.extra.get("accent")}))
                .collect::<Vec<_>>()}),
            ),
            ("retake", "list") => {
                let notes = notes::selected(call, project, selection)?;
                let item = notes::one(&notes)?;
                Ok(json!({"note_id":item.note.uuid,"takes":TakeSet::from_extra(&item.note.extra)?}))
            }
            ("phoneme", _) => {
                let selected = notes::selected(call, project, selection)?;
                let mut planner = Planner::default();
                let plan = planner.update(project, 0, |_| None);
                let requested: BTreeMap<_, _> = selected
                    .iter()
                    .map(|item| (item.reference.id, &item.reference))
                    .collect();
                let mut pronunciations = BTreeMap::new();
                for (id, reference) in requested {
                    let parsed = (|| {
                        let request = plan
                            .requests
                            .get(&OwnerId(id))
                            .ok_or_else(|| invalid("Group has no unmuted visible score"))?;
                        let track = project
                            .tracks
                            .iter()
                            .find(|track| track.references().any(|item| item.id == id))
                            .expect("selected track");
                        let settings = resources::settings()?;
                        let key = self
                            .offline
                            .library(&settings)
                            .catalog
                            .resolve(&track.main_ref.database, &reference.database)
                            .map_err(|error| Failure::new("VOICE_NOT_FOUND", error))?
                            .ok_or_else(|| {
                                Failure::new(
                                    "VOICE_NOT_ASSIGNED",
                                    "Assign an installed voice to resolve automatic pronunciation",
                                )
                            })?;
                        let inventory = self.offline.inventory(&settings, &key)?;
                        let config = inventory.configuration.as_ref().ok_or_else(|| {
                            Failure::new("VOICE_RESOURCE", "Voice configuration missing")
                        })?;
                        let voice = inventory
                            .prepare()
                            .map_err(|error| Failure::new("VOICE_RESOURCE", error.to_string()))?;
                        let frontend = LyricFrontend::new(
                            resources::dictionary_root(&settings)?,
                            &config.default_language,
                            &config.supported_languages,
                        )
                        .map_err(|error| Failure::new("DICTIONARY_RESOURCE", error.to_string()))?;
                        let phrase = score::prepare_owner(&request.input, &frontend, &voice)
                            .map_err(|error| Failure::new("LYRIC_UNRESOLVED", error.to_string()))?;
                        let mut sources = BTreeMap::<u64, BTreeMap<usize, Value>>::new();
                        for phone in phrase.records.iter().flat_map(|record| &record.phones) {
                            if let Some(source) = phone.source {
                                sources.entry(source.note_id).or_default().insert(source.phone_index, json!({
                                    "index":source.phone_index,"token":phone.token,"language":phone.language,"attributes":phone.attributes
                                }));
                            }
                        }
                        Ok::<_, Failure>(sources)
                    })();
                    pronunciations.insert(id, parsed);
                }
                let mut mapping = Vec::new();
                for item in &selected {
                    let (phones, diagnostic) = match &pronunciations[&item.reference.id] {
                        Ok(sources) => (
                            sources
                                .get(&item.note.id)
                                .map(|values| json!(values.values().collect::<Vec<_>>())),
                            None,
                        ),
                        Err(error) => (
                            None,
                            Some(json!({"code":error.code,"message":error.message})),
                        ),
                    };
                    mapping.push(json!({"note_id":item.note.uuid,"group_ref_id":item.reference.uuid,"lyric":item.note.lyrics,
                        "override":item.note.phonemes,"stored_attributes":PhonemeAttributes::read(&item.note.attributes)?,
                        "resolved_phones":phones,"diagnostic":diagnostic}));
                }
                let payload = json!({"mapping":mapping});
                if call.action == "export" {
                    json_file(call.path("output")?, &payload, call)?;
                }
                Ok(payload)
            }
            _ => Err(invalid("Unknown tuning query")),
        }
    }

    pub(super) fn edit_tuning(
        &mut self,
        call: Call<'_>,
        editor: &mut Editor,
        selection: &Selection,
    ) -> Result<Value> {
        if matches!(
            (call.family, call.action),
            ("phoneme" | "retake", _) | ("parameter", "note")
        ) {
            return self.edit_note_tuning(call, editor, selection);
        }
        let (_, reference) = scope(call, &editor.project, selection)?;
        let reference = reference.clone();
        if call.family == "parameter" && call.action == "voice" {
            let patch: BTreeMap<String, Value> = self.read_json(call.required("input")?)?;
            let item = reference_mut(&mut editor.project, reference.id);
            let voice = item
                .extra
                .entry("voice".into())
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or_else(|| invalid("Voice settings are not an object"))?;
            for (key, value) in patch {
                if !crate::model::VoiceParameter::ALL
                    .iter()
                    .any(|parameter| parameter.key() == key)
                    && !matches!(
                        key.as_str(),
                        "vocalModeInherited" | "vocalModeParams" | "vocalModePreset"
                    )
                {
                    return Err(invalid(format!("Unknown voice setting {key}")));
                }
                if value.is_null() {
                    voice.remove(&key);
                } else if key == "vocalModeParams" {
                    let modes = value
                        .as_object()
                        .ok_or_else(|| invalid("vocalModeParams must be an object"))?;
                    let current = voice
                        .entry(key)
                        .or_insert_with(|| json!({}))
                        .as_object_mut()
                        .ok_or_else(|| invalid("Existing vocalModeParams must be an object"))?;
                    for (key, value) in modes {
                        current.insert(key.clone(), value.clone());
                    }
                } else {
                    voice.insert(key, value);
                }
            }
            VoiceSettings::from_extra(&item.extra)?;
            return Ok(json!({"group_ref_id":reference.uuid,"voice":item.extra.get("voice")}));
        }
        target::edit_reference(&editor.project, &reference, call.flag("shared"))?;
        if call.family == "pitch" {
            let input = self.read(call.required("input")?)?;
            let raw: Value =
                serde_json::from_slice(&input).map_err(|error| invalid(error.to_string()))?;
            let edit: PitchEdit =
                serde_json::from_value(raw.clone()).map_err(|error| invalid(error.to_string()))?;
            let group = editor
                .project
                .group_mut(&reference.group_id)
                .expect("selected group");
            if let Some(curve) = edit.pitch_delta {
                check_range(&curve, Parameter::Pitch.range())?;
                group
                    .parameters
                    .insert(Parameter::Pitch.key().into(), curve);
            } else if raw.get("pitch_delta") == Some(&Value::Null) {
                group.parameters.remove(Parameter::Pitch.key());
            }
            if let Some(controls) = edit.pitch_controls {
                group.pitch_controls = controls;
            }
            if raw.get("system_pitch_delta").is_some() {
                reference_mut(&mut editor.project, reference.id).system_pitch_delta =
                    edit.system_pitch_delta.flatten().map(Arc::new);
            }
            let updated = reference_mut(&mut editor.project, reference.id).clone();
            return Ok(pitch_json(&editor.project, &updated));
        }
        let name = call.required("name")?;
        let target = if let Some(mode) = name
            .strip_prefix("mode:")
            .filter(|mode| !mode.trim().is_empty())
        {
            AutomationTarget::VocalMode(mode.into())
        } else {
            let parameter = Parameter::from_key(name)
                .or(match name {
                    "pitch" => Some(Parameter::Pitch),
                    "vibrato" => Some(Parameter::Vibrato),
                    "tone-shift" => Some(Parameter::ToneShift),
                    _ => None,
                })
                .ok_or_else(|| invalid("Use a parameter key from parameter list, or mode:NAME"))?;
            AutomationTarget::Parameter(parameter)
        };
        let group = editor
            .project
            .group_mut(&reference.group_id)
            .expect("selected group");
        let curve = group.ensure_automation(&target);
        match call.action {
            "set" => {
                curve.replace_points(vec![(0, target::number(call.required("value")?)?)])?;
            }
            "curve" => {
                curve.replace_points(self.read_json(call.required("input")?)?)?;
            }
            "point" => {
                let at: i64 = call
                    .number("at")?
                    .ok_or_else(|| invalid("Expected a local Blick position"))?;
                if call.flag("delete") {
                    curve.replace_range(at, at, &[])?;
                } else {
                    curve.insert(at, target::number(call.required("value")?)?)?;
                }
            }
            "transform" => transform(call, curve)?,
            _ => return Err(invalid("Unknown parameter edit")),
        }
        check_range(curve, target.range())?;
        Ok(json!({"group_ref_id":reference.uuid,"name":target.key(),"curve":curve}))
    }

    fn edit_note_tuning(
        &mut self,
        call: Call<'_>,
        editor: &mut Editor,
        selection: &Selection,
    ) -> Result<Value> {
        let items = notes::editable(call, &editor.project, selection)?;
        if call.family == "parameter" {
            let patch: BTreeMap<String, Value> = self.read_json(call.required("input")?)?;
            let allowed = [
                "expValueX",
                "expValueY",
                "dF0VbrMod",
                "rTone",
                "rIntonation",
                "tNoteOffset",
                "evenSyllableDuration",
                "musicalType",
                "accent",
                "sv1Pitch",
                "detune",
            ];
            if patch.keys().any(|key| !allowed.contains(&key.as_str())) {
                return Err(invalid("Unsupported note performance field"));
            }
            for item in &items {
                let note = notes::note_mut(&mut editor.project, item);
                for (key, value) in &patch {
                    match key.as_str() {
                        "musicalType" => {
                            note.musical_type = value
                                .as_str()
                                .ok_or_else(|| invalid("musicalType requires a string"))?
                                .into()
                        }
                        "accent" => {
                            let value = value
                                .as_str()
                                .filter(|value| matches!(*value, "" | "1" | "2" | "3" | "4" | "5"))
                                .ok_or_else(|| invalid("Accent must be an empty string or 1–5"))?;
                            note.extra.insert(key.clone(), json!(value));
                        }
                        "sv1Pitch" => {
                            note.rynthv_sv1_pitch = serde_json::from_value(value.clone())
                                .map_err(|error| invalid(error.to_string()))?
                        }
                        "detune" => {
                            note.detune = value
                                .as_f64()
                                .map(|value| value as f32)
                                .filter(|value| value.is_finite())
                                .ok_or_else(|| invalid("Detune must be a finite number of cents"))?
                        }
                        "evenSyllableDuration" if !value.is_null() && !value.is_boolean() => {
                            return Err(invalid("evenSyllableDuration must be boolean"));
                        }
                        "tNoteOffset" if !value.is_null() && value.as_f64().is_none() => {
                            return Err(invalid("tNoteOffset must be seconds"));
                        }
                        _ if value.is_null() => {
                            note.attributes.remove(key);
                        }
                        _ => {
                            note.attributes.insert(key.clone(), value.clone());
                        }
                    }
                }
                NotePerformance::from_note(note)?;
            }
            return Ok(
                json!({"affected":items.iter().map(|item| &item.note.uuid).collect::<Vec<_>>()}),
            );
        }
        let item = notes::one(&items)?;
        if call.family == "phoneme" {
            let note = notes::note_mut(&mut editor.project, item);
            if call.action == "set" {
                note.replace_phonemes(call.required("value")?);
            } else {
                let index = call
                    .number::<usize>("index")?
                    .ok_or_else(|| invalid("Missing phone index"))?;
                if !note.phonemes.is_empty() && index >= note.phonemes.split_whitespace().count() {
                    return Err(invalid(
                        "Phone index is beyond this note's explicit phonemes",
                    ));
                }
                let attributes: PhonemeAttributes = self.read_json(call.required("input")?)?;
                PhonemeAttributes::set_at(&mut note.attributes, index, attributes)?;
            }
            return Ok(
                json!({"note_id":note.uuid,"override":note.phonemes,"attributes":PhonemeAttributes::read(&note.attributes)?}),
            );
        }
        let note = notes::note_mut(&mut editor.project, item);
        let mut set = TakeSet::from_extra(&note.extra)?;
        match call.action {
            "create" => {
                let component = match call.required("component")? {
                    "all" => RetakeComponent::All,
                    "duration" => RetakeComponent::Duration,
                    "pitch" => RetakeComponent::Pitch,
                    "timbre" => RetakeComponent::Timbre,
                    _ => RetakeComponent::PitchAndTimbre,
                };
                let id = set
                    .takes
                    .iter()
                    .map(|take| take.id)
                    .max()
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or_else(|| invalid("Retake ID space exhausted"))?;
                set.append(id, component)?;
                if let Some(seed) = call.number::<u32>("seed")? {
                    let take = set.takes.last_mut().expect("appended take");
                    if matches!(component, RetakeComponent::All | RetakeComponent::Duration) {
                        take.seed_duration = seed;
                    }
                    if matches!(
                        component,
                        RetakeComponent::All
                            | RetakeComponent::PitchAndTimbre
                            | RetakeComponent::Pitch
                    ) {
                        take.seed_pitch = seed;
                    }
                    if matches!(
                        component,
                        RetakeComponent::All
                            | RetakeComponent::PitchAndTimbre
                            | RetakeComponent::Timbre
                    ) {
                        take.seed_timbre = seed;
                    }
                }
            }
            "select" => set.select(
                call.number::<u32>("id")?
                    .ok_or_else(|| invalid("Missing retake ID"))?,
            )?,
            "prune" => set.retain_current(),
            _ => return Err(invalid("Unknown retake edit")),
        }
        set.write_extra(&mut note.extra);
        let id = note.uuid.clone();
        if set.active().seed_duration != 0 {
            let track = target::track(&editor.project, Some(&item.track_id))?;
            let settings = resources::settings()?;
            let catalog = &self.offline.library(&settings).catalog;
            if let Ok(Some(voice)) =
                catalog.resolve(&track.main_ref.database, &item.reference.database)
                && let Some(entry) = catalog.entry(&voice)
                && entry
                    .family
                    .is_some_and(|family| family != VoiceFamily::Sv2)
            {
                return Err(invalid(
                    "Duration retakes are supported only by native SV2 models",
                ));
            }
        }
        Ok(json!({"note_id":id,"takes":set}))
    }
}

fn reference_mut(project: &mut Project, id: u64) -> &mut GroupRef {
    project
        .tracks
        .iter_mut()
        .flat_map(|track| std::iter::once(&mut track.main_ref).chain(&mut track.groups))
        .find(|reference| reference.id == id)
        .expect("selected reference")
}
fn check_range(curve: &Automation, (min, max): (f64, f64)) -> Result<()> {
    curve.validate()?;
    if curve
        .points
        .iter()
        .any(|&(_, value)| !(min..=max).contains(&value))
    {
        return Err(invalid(format!(
            "Parameter value must be between {min} and {max}"
        )));
    }
    Ok(())
}
fn transform(call: Call<'_>, curve: &mut Automation) -> Result<()> {
    let start = call.number::<i64>("from")?.unwrap_or(i64::MIN);
    let end = call.number::<i64>("to")?.unwrap_or(i64::MAX);
    if start >= end {
        return Err(invalid("Invalid automation range in local Blick"));
    }
    let finish = end.saturating_sub(1);
    if call.flag("reset") {
        curve.replace_range(start, finish, &[])?;
        return Ok(());
    }
    let scale = call
        .get("scale")
        .map(target::number)
        .transpose()?
        .unwrap_or(1.0);
    let offset = call
        .get("offset")
        .map(target::number)
        .transpose()?
        .unwrap_or(0.0);
    let passes = call.number::<usize>("smooth")?.unwrap_or(0);
    if passes > 100 {
        return Err(invalid("Smoothing accepts at most 100 passes"));
    }
    let mut points: Vec<_> = curve
        .points
        .iter()
        .filter(|(position, _)| (start..end).contains(position))
        .map(|(position, value)| (*position, value.mul_add(scale, offset)))
        .collect();
    for _ in 0..passes {
        let smoothed: Vec<_> = points
            .windows(3)
            .map(|triple| (triple[0].1 + 2.0 * triple[1].1 + triple[2].1) / 4.0)
            .collect();
        for (point, value) in points.iter_mut().skip(1).zip(smoothed) {
            point.1 = value;
        }
    }
    curve.replace_range(start, finish, &points)?;
    Ok(())
}

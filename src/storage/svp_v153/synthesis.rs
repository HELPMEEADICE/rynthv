use crate::model::{
    Extra, GroupRef, Note, PhonemeAttributes, PhonemeParameter, Sv1Pitch, VocalModeComponent,
};
use crate::trf;
use serde_json::{Map, Value, json};

pub(super) fn import_reference(reference: &mut GroupRef) {
    if let Some(database) = reference.database.as_object_mut()
        && database.get("backendType").and_then(Value::as_str) == Some("SVR2AI")
        && database
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| !name.is_empty())
        && !database.contains_key("id")
    {
        // A name-only SV1 assignment selects the installed revision of that voice.
        database.insert("version".into(), "-2".into());
    }
    remove_takes(&mut reference.extra);
    if reference
        .system_pitch_delta
        .as_ref()
        .is_some_and(|curve| curve.points.is_empty())
    {
        reference.system_pitch_delta = None;
    }
    if let Some(audio) = reference.extra.get("audio").and_then(Value::as_object)
        && audio.len() == 2
        && audio.get("filename").and_then(Value::as_str) == Some("")
        && audio.get("duration").and_then(Value::as_f64) == Some(0.0)
    {
        reference.extra.remove("audio");
    }
    if let Some(modes) = reference
        .extra
        .get_mut("voice")
        .and_then(Value::as_object_mut)
        .and_then(|voice| voice.get_mut("vocalModeParams"))
        .and_then(Value::as_object_mut)
    {
        for value in modes.values_mut() {
            if let Value::Number(weight) = value {
                let key = VocalModeComponent::Timbre.key();
                *value = json!({(key): weight});
            }
        }
    }
}

pub(super) fn import_note(note: &mut Note) -> Result<(), String> {
    remove_takes(&mut note.extra);
    let generated = match note.extra.remove("systemAttributes") {
        Some(Value::Object(fields)) => fields,
        None => Map::new(),
        Some(_) => return Err(crate::tr!("系统音符参数必须为对象").into()),
    };
    let instant = note
        .extra
        .remove("instantMode")
        .as_ref()
        .and_then(Value::as_bool)
        == Some(true);
    let mut number = |key| {
        note.attributes
            .remove(key)
            .or_else(|| generated.get(key).cloned())
            .map(|value| {
                value
                    .as_f64()
                    .ok_or_else(|| trf!("音高参数 {} 必须为数值", key))
            })
            .transpose()
    };
    let pitch = Sv1Pitch {
        generated: instant,
        transition_offset: number("tF0Offset")?,
        transition_left: number("tF0Left")?,
        transition_right: number("tF0Right")?,
        depth_left: number("dF0Left")?,
        depth_right: number("dF0Right")?,
        vibrato_delay: number("tF0VbrStart")?,
        vibrato_fade_in: number("tF0VbrLeft")?,
        vibrato_fade_out: number("tF0VbrRight")?,
        vibrato_depth: number("dF0Vbr")?,
        vibrato_phase: number("pF0Vbr")?,
        vibrato_frequency: number("fF0Vbr")?,
    };
    if pitch != Sv1Pitch::default() {
        note.rynthv_sv1_pitch = Some(pitch);
    }
    note.attributes.remove("dF0Jitter");
    for key in [
        "evenSyllableDuration",
        "languageOverride",
        "phonesetOverride",
        "tNoteOffset",
        "dur",
        "strength",
    ] {
        if let Some(value) = generated.get(key) {
            note.attributes
                .entry(key.into())
                .or_insert_with(|| value.clone());
        }
    }
    import_phoneme_scales(&mut note.attributes, "dur", PhonemeParameter::DurationScale)?;
    import_phoneme_scales(&mut note.attributes, "strength", PhonemeParameter::Strength)
}

fn import_phoneme_scales(
    attributes: &mut Extra,
    key: &str,
    parameter: PhonemeParameter,
) -> Result<(), String> {
    if let Some(value) = attributes.remove(key) {
        let scales: Vec<f64> = serde_json::from_value(value)
            .map_err(|error| trf!("音素参数 {} 无效：{}", key, error))?;
        if scales.len() > 4096 {
            return Err(crate::tr!("音素参数数量超出支持范围").into());
        }
        for (index, scale) in scales.into_iter().enumerate() {
            if scale != 1.0 {
                let value = match parameter {
                    PhonemeParameter::Strength => scale - 1.0,
                    _ => scale,
                };
                PhonemeAttributes::update_at(attributes, index, parameter, Some(value))?;
            }
        }
    }
    Ok(())
}

fn remove_takes(extra: &mut Extra) {
    // SV1's take IDs and expression values are not model seeds.
    extra.remove("pitchTakes");
    extra.remove("timbreTakes");
}

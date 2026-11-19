use crate::model::{
    Extra, GroupRef, Note, PhonemeAttributes, PhonemeParameter, VocalModeComponent,
};
use serde_json::{Value, json};

pub(super) fn import_reference(reference: &mut GroupRef) {
    remove_takes(&mut reference.extra);
    // The previous renderer's generated pitch is rebuilt from the imported score.
    reference.extra.remove("systemPitchDelta");
    if reference.extra.get("audio") == Some(&json!({"filename": "", "duration": 0})) {
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
    note.extra.remove("instantMode");
    note.extra.remove("systemAttributes");
    // These editor-specific controls have no equivalent in the current renderer.
    for key in [
        "tF0Offset",
        "tF0Left",
        "tF0Right",
        "dF0Left",
        "dF0Right",
        "tF0VbrStart",
        "tF0VbrLeft",
        "tF0VbrRight",
        "dF0Vbr",
        "pF0Vbr",
        "fF0Vbr",
        "dF0Jitter",
        "strength",
    ] {
        note.attributes.remove(key);
    }
    if let Some(value) = note.attributes.remove("dur") {
        let scales: Vec<f64> =
            serde_json::from_value(value).map_err(|error| format!("音素时长倍率无效：{error}"))?;
        for (index, scale) in scales.into_iter().enumerate() {
            if scale != 1.0 {
                PhonemeAttributes::update_at(
                    &mut note.attributes,
                    index,
                    PhonemeParameter::DurationScale,
                    Some(scale),
                )?;
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

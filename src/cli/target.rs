use super::*;
use crate::model::{GroupRef, Note, Project, QUARTER, TimeAxis, Track, beats};

pub(super) fn track<'a>(project: &'a Project, selector: Option<&str>) -> Result<&'a Track> {
    if let Some(selector) = selector {
        if let Some(track) = project
            .tracks
            .iter()
            .find(|track| track.main_ref.uuid.eq_ignore_ascii_case(selector))
        {
            return Ok(track);
        }
        let mut tracks = project.tracks.iter().filter(|track| track.name == selector);
        if let Some(track) = tracks.next() {
            if tracks.next().is_none() {
                return Ok(track);
            }
            return Err(Failure::new(
                "AMBIGUOUS_TRACK",
                "Multiple tracks use this name. Use a track UUID.",
            )
            .details(json!({"track": selector})));
        }
        return Err(
            Failure::new("TRACK_NOT_FOUND", "Track UUID or name was not found")
                .details(json!({"track": selector})),
        );
    }
    project
        .tracks
        .iter()
        .find(|track| !track.is_instrumental())
        .or_else(|| project.tracks.first())
        .ok_or_else(|| invalid("The project has no tracks"))
}

pub(super) fn reference<'a>(track: &'a Track, selector: Option<&str>) -> Result<&'a GroupRef> {
    let Some(selector) = selector else {
        return Ok(&track.main_ref);
    };
    track
        .references()
        .find(|reference| reference.uuid.eq_ignore_ascii_case(selector))
        .ok_or_else(|| {
            Failure::new(
                "GROUP_NOT_FOUND",
                "Group reference was not found on that track",
            )
            .details(json!({"track_id": track.main_ref.uuid, "group_ref_id": selector}))
        })
}

pub(super) fn scope_track<'a>(
    project: &'a Project,
    track_id: Option<&str>,
    group_id: Option<&str>,
) -> Result<&'a Track> {
    if track_id.is_none()
        && let Some(group_id) = group_id
    {
        return project
            .tracks
            .iter()
            .find(|track| {
                track
                    .references()
                    .any(|reference| reference.uuid.eq_ignore_ascii_case(group_id))
            })
            .ok_or_else(|| {
                Failure::new("GROUP_NOT_FOUND", "Group reference was not found")
                    .details(json!({"group_ref_id":group_id}))
            });
    }
    track(project, track_id)
}

pub(super) fn edit_reference(project: &Project, reference: &GroupRef, shared: bool) -> Result<()> {
    if shared
        || project
            .tracks
            .iter()
            .flat_map(Track::references)
            .filter(|other| other.group_id == reference.group_id)
            .take(2)
            .count()
            <= 1
    {
        return Ok(());
    }
    Err(Failure::new(
        "SHARED_GROUP",
        "Use group independent first, or pass --shared to edit all linked instances.",
    )
    .details(json!({"group_ref_id": reference.uuid, "group_id": reference.group_id})))
}

pub(super) fn signature(text: &str) -> Result<(u32, u32)> {
    let (n, d) = text
        .split_once('/')
        .ok_or_else(|| invalid("Expected a numerator/denominator pair"))?;
    Ok((
        n.parse().map_err(|_| invalid("Invalid numerator"))?,
        d.parse().map_err(|_| invalid("Invalid denominator"))?,
    ))
}

pub(super) fn number(value: &str) -> Result<f64> {
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| invalid(format!("Expected a finite number, got {value:?}")))
}

fn tick_number(value: f64) -> Result<i64> {
    if !(0.0..i64::MAX as f64).contains(&value) {
        return Err(invalid(
            "Position is outside the supported nonnegative range",
        ));
    }
    Ok(value.round() as i64)
}

pub(super) fn position(time: &TimeAxis, unit: &str, value: &str) -> Result<i64> {
    match unit {
        "blick" => value
            .parse()
            .ok()
            .filter(|&value: &i64| (0..i64::MAX).contains(&value))
            .ok_or_else(|| invalid("Expected a nonnegative integer Blick position")),
        "beat" => tick_number(number(value)? * QUARTER as f64),
        "second" => {
            let seconds = number(value)?;
            if seconds < time.seconds_at(0) || seconds > time.seconds_at(i64::MAX - 1) {
                return Err(invalid("Second position is outside the project time axis"));
            }
            Ok(time.position_at(seconds))
        }
        "bar" => {
            let (bar, beat) = value.split_once(':').unwrap_or((value, "1"));
            let bar = bar
                .parse::<i32>()
                .ok()
                .and_then(|bar| bar.checked_sub(1))
                .filter(|&bar| bar >= 0)
                .ok_or_else(|| invalid("Bar numbers begin at 1"))?;
            let meter = time.meter_at(bar);
            let beat = number(beat)?;
            if !(1.0..f64::from(meter.numerator) + 1.0).contains(&beat) {
                return Err(invalid("Beat is outside the bar's meter"));
            }
            time.bar_position(bar)
                .checked_add(tick_number(
                    (beat - 1.0) * QUARTER as f64 * 4.0 / f64::from(meter.denominator),
                )?)
                .ok_or_else(|| invalid("Bar position overflow"))
        }
        _ => Err(invalid("Unknown time unit")),
    }
}

pub(super) fn duration(time: &TimeAxis, unit: &str, at: i64, value: &str) -> Result<i64> {
    let length = match unit {
        "second" => {
            position(
                time,
                unit,
                &(time.seconds_at(at) + number(value)?).to_string(),
            )? - at
        }
        "bar" => {
            let bars = value
                .parse::<i32>()
                .ok()
                .filter(|&value| value > 0)
                .ok_or_else(|| invalid("Bar duration must be a positive integer"))?;
            let (bar, _) = time.bar_at(at);
            let end = bar
                .checked_add(bars)
                .ok_or_else(|| invalid("Bar duration overflow"))?;
            time.bar_position(end) - time.bar_position(bar)
        }
        _ => position(time, unit, value)?,
    };
    if length <= 0 || at.checked_add(length).is_none() {
        return Err(invalid("Duration must be positive and fit the timeline"));
    }
    Ok(length)
}

pub(super) fn pitch(value: &str) -> Result<i32> {
    let parsed = value.parse::<i32>().ok().or_else(|| {
        let mut text = value.chars();
        let letter = match text.next()?.to_ascii_uppercase() {
            'C' => 0,
            'D' => 2,
            'E' => 4,
            'F' => 5,
            'G' => 7,
            'A' => 9,
            'B' => 11,
            _ => return None,
        };
        let rest = text.as_str();
        let (key, octave) = if let Some(octave) = rest.strip_prefix(['#', '♯']) {
            (letter + 1, octave)
        } else if let Some(octave) = rest.strip_prefix(['b', '♭']) {
            (letter - 1, octave)
        } else {
            (letter, rest)
        };
        octave
            .parse::<i32>()
            .ok()?
            .checked_add(1)?
            .checked_mul(12)?
            .checked_add(key)
    });
    parsed
        .filter(|value| (0..=127).contains(value))
        .ok_or_else(|| {
            invalid(format!(
                "Pitch {value:?} must be a MIDI number 0–127 or a note name"
            ))
        })
}

pub(super) fn note_json(note: &Note, reference: &GroupRef, time: &TimeAxis) -> Value {
    let global = note.onset.saturating_add(reference.blick_offset);
    let end = global.saturating_add(note.duration);
    json!({
        "id": note.uuid, "group_ref_id": reference.uuid, "group_id": reference.group_id,
        "onset_blick": global, "onset_beat": beats(global), "onset_seconds": time.seconds_at(global),
        "local_onset_blick": note.onset, "duration_blick": note.duration,
        "duration_seconds": time.seconds_at(end) - time.seconds_at(global),
        "midi_pitch": note.pitch.saturating_add(reference.pitch_offset), "detune_cents": note.detune,
        "lyric": note.lyrics, "phonemes": note.phonemes, "muted": note.muted(), "musical_type": note.musical_type
    })
}

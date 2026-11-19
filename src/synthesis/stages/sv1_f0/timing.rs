//! Separate note and phone clocks for the SV1 NOFS DDS frontend.
//!
//! The acoustic wrapper truncates another clock, so its TimedPhone.frames
//! must not be reused here. Both DDS grids end at the same phrase frame.

use crate::synthesis::{
    stages::f0::frontend::FrontendNote,
    voice::{Result, VoiceError},
};

pub(super) fn duration_seconds(note: &FrontendNote) -> Result<f64> {
    if note.parts == 0
        || note.part_index >= note.parts
        || !note.duration_seconds.is_finite()
        || note.duration_seconds < 0.0
    {
        return Err(VoiceError::new("Invalid native DDS note duration"));
    }
    let mut duration = note.duration_seconds / f64::from(note.parts);
    for &extra in &note.appended_durations {
        if !extra.is_finite() || extra < 0.0 {
            return Err(VoiceError::new("Invalid native DDS linked note duration"));
        }
        duration += extra;
    }
    if !duration.is_finite() {
        return Err(VoiceError::new("Native DDS note duration overflow"));
    }
    Ok(duration)
}

pub(super) fn note_frame_counts(notes: &[FrontendNote], step: f32) -> Result<Vec<usize>> {
    let mut elapsed = 0.0;
    let mut previous = 0;
    let inverse = f64::from(step).recip();
    notes
        .iter()
        .map(|note| {
            elapsed += duration_seconds(note)?;
            let end = frame((elapsed * inverse).round())?;
            let count = end - previous;
            previous = end;
            Ok(count)
        })
        .collect()
}

pub(super) fn phone_frame_counts(
    seconds: &[f64],
    step: f32,
    phrase_frames: usize,
) -> Result<Vec<usize>> {
    let mut elapsed = 0.0_f32;
    let mut previous = 0;
    seconds
        .iter()
        .enumerate()
        .map(|(index, &duration)| {
            if !duration.is_finite() || duration < 0.0 || duration > f64::from(f32::MAX) {
                return Err(VoiceError::new("Invalid native DDS phone duration"));
            }
            elapsed += duration as f32;
            let end = frame(f64::from((elapsed / step).round()))?;
            // Rounded phone durations can drift slightly from the note clock.
            // Anchor the phrase end at the trailing silence.
            let end = if index + 1 == seconds.len() {
                phrase_frames
            } else {
                end.min(phrase_frames)
            };
            let count = end - previous;
            previous = end;
            Ok(count)
        })
        .collect()
}

fn frame(value: f64) -> Result<usize> {
    if !value.is_finite() || value < 0.0 || value >= f64::from(i32::MAX) {
        return Err(VoiceError::new(
            "Native DDS frame count exceeds its native range",
        ));
    }
    Ok(value as usize)
}

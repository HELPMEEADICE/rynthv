use crate::{
    model::{Automation, NoteMode, Sv1Pitch},
    synthesis::{
        plan::{GlobalBlick, OwnerInput, TempoMapping},
        voice::{Result, VoiceError},
    },
};
use std::{f64::consts, sync::Arc};

#[derive(Clone, Debug)]
pub(super) struct Pitch {
    notes: Vec<Note>,
    cached: Option<Arc<Automation>>,
    offset: i64,
}

#[derive(Clone, Debug)]
struct Note {
    begin: f64,
    end: f64,
    key: f64,
    settings: Option<Sv1Pitch>,
    transition: Option<Transition>,
}

#[derive(Clone, Debug)]
struct Transition {
    boundary: f64,
    start: f64,
    pivot: f64,
    end: f64,
    from: f64,
    to: f64,
    depth_left: f64,
    depth_right: f64,
}

impl Pitch {
    pub(super) fn new(input: &OwnerInput, tuning: f64) -> Result<Option<Self>> {
        if input.notes.iter().all(|note| note.sv1_pitch.is_none()) {
            return Ok(None);
        }
        let mut notes = Vec::new();
        let mut tempo = input.tempo.cursor();
        for note in &input.notes {
            if super::super::options::muted(&note.attributes)? {
                continue;
            }
            let start = note
                .onset
                .0
                .checked_add(input.content_offset.0)
                .ok_or_else(|| VoiceError::new("一代音高的音符位置溢出"))?;
            let end = start
                .checked_add(note.duration)
                .ok_or_else(|| VoiceError::new("一代音高的音符结束位置溢出"))?;
            let key = f64::from(note.pitch)
                + f64::from(input.pitch_offset)
                + f64::from(note.detune) / 100.0
                + tuning;
            let mode = NoteMode::parse(&note.musical_type).map_err(VoiceError::new)?;
            let settings = note
                .sv1_pitch
                .filter(|settings| mode == NoteMode::Singing || settings.generated);
            notes.push(Note {
                begin: tempo.seconds_at(GlobalBlick(start)),
                end: tempo.seconds_at(GlobalBlick(end)),
                key,
                settings,
                transition: None,
            });
        }
        notes.sort_by(|a, b| a.begin.total_cmp(&b.begin));
        for index in 1..notes.len() {
            notes[index].transition = Transition::between(&notes[index - 1], &notes[index]);
        }
        Ok(Some(Self {
            notes,
            cached: input.system_pitch_delta.clone(),
            offset: input.content_offset.0,
        }))
    }

    pub(super) fn memory_bytes(&self) -> usize {
        self.notes.capacity() * std::mem::size_of::<Note>()
    }

    pub(super) fn fill(
        &self,
        tempo: &TempoMapping,
        origin: f64,
        step: f64,
        frames: &mut [f32],
    ) -> Result<()> {
        let cached = self
            .cached
            .as_deref()
            .map(|curve| curve.sampler(0.0))
            .transpose()
            .map_err(VoiceError::new)?;
        let mut index = self
            .notes
            .partition_point(|note| note.begin <= origin)
            .saturating_sub(1);
        for (frame, value) in frames.iter_mut().enumerate() {
            let time = origin + frame as f64 * step;
            while let Some(next) = self.notes.get(index + 1) {
                let middle = (self.notes[index].end + next.begin) * 0.5;
                if time < middle {
                    break;
                }
                index += 1;
            }
            let Some(note) = self.notes.get(index) else {
                break;
            };
            if value.is_finite() {
                continue;
            }
            // Preserve pre-onset and release pitch for voiced phonemes around the note.
            if time < note.begin - 0.15 || time >= note.end + 0.15 {
                continue;
            }
            let transition = note
                .transition
                .as_ref()
                .and_then(|transition| transition.at(time))
                .or_else(|| self.notes.get(index + 1)?.transition.as_ref()?.at(time));
            if transition.is_none() && note.settings.is_none() {
                continue;
            }
            let mut pitch = transition.unwrap_or(note.key);
            if note.settings.is_some_and(|settings| settings.generated)
                && let (Some(cached), Some(curve)) = (&cached, &self.cached)
            {
                let local = tempo.position_at(time).0.saturating_sub(self.offset);
                if let Some((first, last)) = curve.points.first().zip(curve.points.last())
                    && (first.0..=last.0).contains(&local)
                {
                    pitch += cached.sample(local).map_err(VoiceError::new)? / 100.0;
                }
            }
            if let Some(settings) = note.settings {
                pitch += vibrato(settings, note, time);
            }
            if !pitch.is_finite() || !((pitch as f32).is_finite()) {
                return Err(VoiceError::new("一代音高曲线产生了无效数值"));
            }
            *value = pitch as f32;
        }
        Ok(())
    }
}

impl Transition {
    fn between(left: &Note, right: &Note) -> Option<Self> {
        let settings = right.settings.or(left.settings)?;
        if right.begin > left.end {
            return None;
        }
        let pivot =
            (right.begin + settings.transition_offset.unwrap_or(0.0)).clamp(left.begin, right.end);
        let before = settings
            .transition_left
            .unwrap_or(0.1)
            .min(left.end - left.begin);
        let after = settings
            .transition_right
            .unwrap_or(0.1)
            .min(right.end - right.begin);
        Some(Self {
            boundary: right.begin,
            start: (pivot - before).max(left.begin),
            pivot,
            end: (pivot + after).min(right.end),
            from: left.key,
            to: right.key,
            depth_left: settings.depth_left.unwrap_or(0.0),
            depth_right: settings.depth_right.unwrap_or(0.0),
        })
    }

    fn at(&self, time: f64) -> Option<f64> {
        if time < self.start.min(self.boundary) || time >= self.end.max(self.boundary) {
            return None;
        }
        if time < self.start {
            return Some(self.from);
        }
        if time >= self.end {
            return Some(self.to);
        }
        let left = time < self.pivot;
        let (start, end, depth) = if left {
            (self.start, self.pivot, -self.depth_left)
        } else {
            (self.pivot, self.end, self.depth_right)
        };
        let fraction = (time - start) / (end - start);
        let weight = if left {
            smooth(fraction) * 0.5
        } else {
            0.5 + smooth(fraction) * 0.5
        };
        Some(self.from + (self.to - self.from) * weight + (fraction * consts::PI).sin() * depth)
    }
}

fn vibrato(settings: Sv1Pitch, note: &Note, time: f64) -> f64 {
    let depth = settings.vibrato_depth.unwrap_or(0.0);
    let elapsed = time - note.begin - settings.vibrato_delay.unwrap_or(0.25);
    let remaining = note.end - time;
    if depth == 0.0 || elapsed < 0.0 || remaining <= 0.0 {
        return 0.0;
    }
    let gain = |time: f64, width: Option<f64>| match width.unwrap_or(0.1) {
        width if width > 0.0 => smooth((time / width).min(1.0)),
        _ => 1.0,
    };
    let phase = elapsed * settings.vibrato_frequency.unwrap_or(5.5) * consts::TAU
        + settings.vibrato_phase.unwrap_or(0.0);
    phase.sin()
        * depth
        * gain(elapsed, settings.vibrato_fade_in)
        * gain(remaining, settings.vibrato_fade_out)
}

fn smooth(fraction: f64) -> f64 {
    (1.0 - (fraction * consts::PI).cos()) * 0.5
}

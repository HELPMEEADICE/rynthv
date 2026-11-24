use crate::model::{Project, QUARTER, Track};
use midly::{
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind,
    num::{u4, u7, u15, u24, u28},
};
use std::path::Path;

const PPQN: u16 = 960;

pub fn save(
    path: &Path,
    project: &Project,
    selected_track: Option<u64>,
    overwrite: bool,
    dry_run: bool,
) -> Result<usize, String> {
    let mut conductor = Vec::new();
    for tempo in &project.time.tempo {
        let micros = (60_000_000.0 / tempo.bpm).round();
        if !(1.0..=16_777_215.0).contains(&micros) {
            return Err(format!("BPM {} is outside the MIDI tempo range", tempo.bpm));
        }
        conductor.push((
            tick(tempo.position),
            0,
            TrackEventKind::Meta(MetaMessage::Tempo(u24::new(micros as u32))),
        ));
    }
    for meter in &project.time.meter {
        conductor.push((
            tick(project.time.bar_position(meter.index)),
            0,
            TrackEventKind::Meta(MetaMessage::TimeSignature(
                meter.numerator as u8,
                meter.denominator.trailing_zeros() as u8,
                24,
                8,
            )),
        ));
    }
    let mut tracks = vec![events(conductor)?];
    for track in project
        .tracks
        .iter()
        .filter(|track| !track.is_instrumental() && selected_track.is_none_or(|id| id == track.id))
    {
        if let Some(track) = vocal_track(project, track)? {
            tracks.push(track);
        }
    }
    let track_count = tracks.len() - 1;
    if tracks.len() > usize::from(u16::MAX) {
        return Err("Too many tracks for a Standard MIDI File".into());
    }
    if track_count == 0 {
        return Err("No vocal notes selected for MIDI export".into());
    }
    if !dry_run {
        let midi = Smf {
            header: Header::new(Format::Parallel, Timing::Metrical(u15::new(PPQN))),
            tracks,
        };
        crate::storage::atomic_write_policy(path, overwrite, |writer| {
            midi.write_std(writer).map_err(|error| error.to_string())
        })?;
    }
    Ok(track_count)
}

fn vocal_track<'a>(
    project: &'a Project,
    track: &'a Track,
) -> Result<Option<Vec<TrackEvent<'a>>>, String> {
    let mut output = vec![(
        0,
        0,
        TrackEventKind::Meta(MetaMessage::TrackName(track.name.as_bytes())),
    )];
    for reference in track.references().filter(|reference| !reference.mute) {
        let group = project.group(&reference.group_id).ok_or("Missing group")?;
        for note in group.notes.iter().filter(|note| !note.muted()) {
            let Some((from, to)) = reference.note_range(note) else {
                continue;
            };
            let key = note
                .pitch
                .checked_add(reference.pitch_offset)
                .filter(|pitch| (0..=127).contains(pitch))
                .ok_or("Transposed pitch is outside MIDI range")?;
            let key = u7::new(key as u8);
            let start = tick(from);
            let end = tick(to).max(start + 1);
            let channel = u4::new(0);
            output.push((
                start,
                2,
                TrackEventKind::Meta(MetaMessage::Lyric(note.lyrics.as_bytes())),
            ));
            output.push((
                start,
                3,
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::NoteOn {
                        key,
                        vel: u7::new(100),
                    },
                },
            ));
            output.push((
                end,
                1,
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::NoteOff {
                        key,
                        vel: u7::new(0),
                    },
                },
            ));
        }
    }
    if output.len() == 1 {
        return Ok(None);
    }
    events(output).map(Some)
}
fn tick(blick: i64) -> u64 {
    ((i128::from(blick.max(0)) * i128::from(PPQN) + i128::from(QUARTER) / 2) / i128::from(QUARTER))
        as u64
}
fn events(mut events: Vec<(u64, u8, TrackEventKind<'_>)>) -> Result<Vec<TrackEvent<'_>>, String> {
    events.sort_by_key(|event| (event.0, event.1));
    let mut position = 0;
    let mut result = Vec::with_capacity(events.len() + 1);
    for (time, _, kind) in events {
        let delta = u32::try_from(time - position)
            .ok()
            .and_then(u28::try_from)
            .ok_or("Gap exceeds the MIDI delta time limit")?;
        result.push(TrackEvent { delta, kind });
        position = time;
    }
    result.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    Ok(result)
}

//! Standard MIDI files are imported as editable notes, not instrument playback.
mod export;
mod instruments;
mod time;
mod track;
pub use export::save as save_project;

use crate::model::TimeAxis;
use crate::platform::fs::File;
use crate::trf;
use midly::{Format, Timing};
use std::{io::Read, path::Path};

const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TRACKS: usize = 256;
const MAX_ENTRIES: usize = 4096;
const MAX_EVENTS: usize = 2_000_000;
const MAX_NOTES: usize = 200_000;
const MAX_TEXT: usize = 4096;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MidiFile {
    pub tracks: Vec<MidiTrack>,
    pub time: TimeAxis,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MidiTrack {
    pub name: String,
    /// Zero-based index of the source SMF track.
    pub source_index: usize,
    /// Zero-based MIDI channel; empty conductor tracks have no channel.
    pub channel: Option<u8>,
    pub instrument: String,
    pub notes: Vec<MidiNote>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MidiNote {
    pub onset: i64,
    pub duration: i64,
    pub pitch: i32,
    pub lyrics: Option<String>,
}

pub fn load(path: &Path) -> Result<MidiFile, String> {
    let file = File::open(path).map_err(|e| trf!("无法打开 MIDI 文件：{}", e))?;
    let metadata = file
        .metadata()
        .map_err(|e| trf!("无法读取 MIDI 文件：{}", e))?;
    if !metadata.is_file() {
        return Err(trf!("MIDI 路径 {} 不是普通文件", path.display()));
    }
    if metadata.len() > MAX_BYTES {
        return Err(crate::tr!("MIDI 文件不能超过 64 MiB").into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| trf!("无法读取 MIDI 文件：{}", e))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(crate::tr!("MIDI 文件不能超过 64 MiB").into());
    }
    parse(&bytes)
}

pub fn parse(bytes: &[u8]) -> Result<MidiFile, String> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err(crate::tr!("MIDI 文件不能超过 64 MiB").into());
    }
    let (header, source_tracks) =
        midly::parse(bytes).map_err(|e| trf!("MIDI 文件格式无效或不完整：{}", e))?;
    if header.format == Format::Sequential {
        return Err(crate::tr!("暂不支持 Format 2 MIDI；请先转换为 Format 0 或 Format 1").into());
    }
    let ppqn = match header.timing {
        Timing::Metrical(ppqn) if ppqn.as_int() > 0 => ppqn.as_int(),
        Timing::Metrical(_) => return Err(crate::tr!("MIDI 的每四分音符刻度数不能为 0").into()),
        Timing::Timecode(_, _) => {
            return Err(crate::tr!("暂不支持使用 SMPTE 时间码计时的 MIDI").into());
        }
    };
    let declared_tracks = source_tracks.size_hint().0;
    if declared_tracks > MAX_TRACKS {
        return Err(trf!("MIDI 最多支持 {} 个源音轨", MAX_TRACKS));
    }
    let mut budget = Budget::default();
    let mut time = time::TimeEvents::default();
    let mut warnings = Vec::new();
    let mut tracks = Vec::new();
    let mut count = 0;
    for (index, source) in source_tracks.enumerate() {
        count += 1;
        if count > MAX_TRACKS {
            return Err(trf!("MIDI 最多支持 {} 个源音轨", MAX_TRACKS));
        }
        let events = source.map_err(|e| trf!("MIDI 音轨 {} 格式无效：{}", index + 1, e))?;
        let parsed = track::parse(index, events, ppqn, &mut budget, &mut time, &mut warnings)?;
        if tracks.len() + parsed.len() > MAX_ENTRIES {
            return Err(trf!("MIDI 拆分通道后超过 {} 条音轨，无法导入", MAX_ENTRIES));
        }
        tracks.extend(parsed);
    }
    if count != declared_tracks
        || count == 0
        || (header.format == Format::SingleTrack && count != 1)
    {
        return Err(crate::tr!("MIDI 实际音轨数量与文件头不一致").into());
    }
    Ok(MidiFile {
        tracks,
        time: time.finish(ppqn, &mut warnings)?,
        warnings,
    })
}

#[derive(Default)]
struct Budget {
    events: usize,
    notes: usize,
}

impl Budget {
    fn event(&mut self) -> Result<(), String> {
        self.events += 1;
        if self.events > MAX_EVENTS {
            return Err(trf!("MIDI 事件数超过 {}，无法导入", MAX_EVENTS));
        }
        Ok(())
    }

    fn note(&mut self) -> Result<(), String> {
        self.notes += 1;
        if self.notes > MAX_NOTES {
            return Err(trf!("MIDI 音符数超过 {}，无法导入", MAX_NOTES));
        }
        Ok(())
    }
}

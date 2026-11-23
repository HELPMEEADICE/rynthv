use crate::model::*;
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::Path,
};

mod identity;
mod svp_v153;

const MAX_PROJECT_MIB: u64 = 64;
const MAX_PROJECT_BYTES: u64 = MAX_PROJECT_MIB * 1024 * 1024;

pub struct LoadedProject {
    pub project: Project,
    pub imported: bool,
}

pub fn load(path: &Path) -> Result<LoadedProject, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err(format!("工程路径 {} 不是普通文件", path.display()));
    }
    check_project_size(metadata.len())?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_PROJECT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    check_project_size(bytes.len() as u64)?;
    let mut project: Project =
        serde_json::from_slice(&bytes).map_err(|e| format!("SVP 解析失败：{e}"))?;
    let imported = match project.version {
        SVP_VERSION => false,
        svp_v153::VERSION => true,
        version => {
            return Err(format!(
                "不支持 SVP 版本 {version}，可打开 {SVP_VERSION} 或导入 {}",
                svp_v153::VERSION
            ));
        }
    };
    validate_time(&project.time)?;
    if imported {
        svp_v153::import(&mut project)?;
    }
    validate_content(&project)?;
    let base = std::path::absolute(path).map_err(|e| e.to_string())?;
    let parent = base
        .parent()
        .ok_or_else(|| format!("路径 {} 缺少父目录", path.display()))?;
    for track in &mut project.tracks {
        if let Some(clip) = track.instrumental.as_mut().and_then(|t| t.clip.as_mut())
            && clip.path.is_relative()
        {
            clip.path = resolve_relative_clip(parent, &clip.path);
        }
    }
    project.tracks.sort_by_key(|t| t.disp_order);
    for group in project
        .library
        .iter_mut()
        .chain(project.tracks.iter_mut().map(|t| &mut t.main_group))
    {
        group.notes.sort_by_key(|n| n.onset);
    }
    Ok(LoadedProject { project, imported })
}

pub fn save(path: &Path, project: &Project) -> Result<(), String> {
    if project.version != SVP_VERSION {
        return Err(format!("只能保存为 SVP {SVP_VERSION} 工程"));
    }
    validate_time(&project.time)?;
    validate_content(project)?;
    let mut ordered = project.clone();
    let base = std::path::absolute(path).map_err(|e| e.to_string())?;
    let parent = base
        .parent()
        .ok_or_else(|| format!("路径 {} 缺少父目录", path.display()))?;
    for track in &mut ordered.tracks {
        if let Some(clip) = track.instrumental.as_mut().and_then(|t| t.clip.as_mut())
            && let Ok(relative) = clip.path.strip_prefix(parent)
            && !relative.as_os_str().is_empty()
        {
            clip.path = portable_relative_path(relative);
        }
    }
    for group in ordered
        .library
        .iter_mut()
        .chain(ordered.tracks.iter_mut().map(|t| &mut t.main_group))
    {
        group.notes.sort_by_key(|n| n.onset);
    }
    let bytes = serde_json::to_vec_pretty(&ordered).map_err(|e| e.to_string())?;
    check_project_size(bytes.len() as u64)?;
    atomic_write(path, &bytes)
}

fn check_project_size(bytes: u64) -> Result<(), String> {
    if bytes > MAX_PROJECT_BYTES {
        return Err(format!("工程超过 {MAX_PROJECT_MIB} MiB 文件大小上限"));
    }
    Ok(())
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if path.as_os_str().to_string_lossy().ends_with(['/', '\\']) {
        return Err(format!("写入 {} 失败：目标路径不是文件", path.display()));
    }
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("写入 {} 失败：路径缺少文件名", path.display()))?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let temp = parent.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        #[cfg(unix)]
        if let Ok(dir) = fs::File::open(parent) {
            dir.sync_all()?;
        }
        Ok::<(), std::io::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(|e| format!("写入 {} 失败：{e}", path.display()))
}

fn portable_relative_path(path: &Path) -> std::path::PathBuf {
    let mut parts = Vec::new();
    for component in path.components() {
        if let std::path::Component::Normal(part) = component {
            parts.push(part.to_string_lossy());
        }
    }
    if parts.is_empty() {
        path.to_owned()
    } else {
        std::path::PathBuf::from(parts.join("/"))
    }
}

fn resolve_relative_clip(parent: &Path, clip_path: &Path) -> std::path::PathBuf {
    #[cfg(not(windows))]
    if let Some(text) = clip_path.to_str()
        && text.contains('\\')
    {
        let mut resolved = parent.to_path_buf();
        for segment in text.split(['/', '\\']).filter(|s| !s.is_empty()) {
            resolved.push(segment);
        }
        return resolved;
    }
    parent.join(clip_path)
}

fn validate_time(time: &TimeAxis) -> Result<(), String> {
    let bad = |s: &str| Err(format!("工程校验失败：{s}"));
    if time.tempo.first().is_none_or(|t| t.position != 0)
        || time.meter.first().is_none_or(|m| m.index != 0)
    {
        return bad("时间轴必须从 0 开始");
    }
    if !time.start_time_seconds.is_finite()
        || time.tempo.iter().any(|t| {
            !t.bpm.is_finite() || t.bpm <= 0.0 || !(60.0 / t.bpm).is_finite() || t.position < 0
        })
        || time
            .tempo
            .windows(2)
            .any(|a| a[0].position >= a[1].position)
    {
        return bad("曲速必须为正数，标记按时间严格递增");
    }
    if time.meter.iter().any(|m| {
        m.index < 0
            || m.numerator == 0
            || m.numerator > 64
            || !m.denominator.is_power_of_two()
            || m.denominator > 64
    }) || time.meter.windows(2).any(|a| a[0].index >= a[1].index)
    {
        return bad("拍号或拍号位置无效");
    }
    Ok(())
}

fn validate_content(p: &Project) -> Result<(), String> {
    let bad = |s: &str| Err(format!("工程校验失败：{s}"));
    identity::validate(p)?;
    if p.tracks.is_empty() {
        return bad("至少需要一条音轨");
    }
    for (name, dictionary) in &p.dictionaries {
        if name.trim().is_empty() || name != name.trim() {
            return bad("发音词典名称不能为空或包含首尾空白");
        }
        if dictionary.entries.iter().any(|(word, phones)| {
            word.trim().is_empty()
                || word != word.trim()
                || phones.split_whitespace().next().is_none()
        }) {
            return bad("发音词典的字词或音素无效");
        }
    }
    struct GroupBounds {
        notes: Option<(i64, i64)>,
        pitches: Option<(i32, i32)>,
        pitch_controls: Option<(i64, i64)>,
    }

    let mut groups = HashMap::new();
    for g in p
        .library
        .iter()
        .chain(p.tracks.iter().map(|t| &t.main_group))
    {
        if g.uuid.is_empty() {
            return bad("音符组 UUID 缺失或重复");
        }
        let mut note_bounds: Option<(i64, i64)> = None;
        let mut pitch_bounds: Option<(i32, i32)> = None;
        for note in &g.notes {
            let Some(note_end) = note.onset.checked_add(note.duration) else {
                return bad("音符时间或 MIDI 音高无效");
            };
            if note.duration <= 0 || !(0..=127).contains(&note.pitch) || !note.detune.is_finite() {
                return bad("音符时间或 MIDI 音高无效");
            }
            note_bounds = Some(match note_bounds {
                Some((min_o, max_e)) => (min_o.min(note.onset), max_e.max(note_end)),
                None => (note.onset, note_end),
            });
            pitch_bounds = Some(match pitch_bounds {
                Some((min_p, max_p)) => (min_p.min(note.pitch), max_p.max(note.pitch)),
                None => (note.pitch, note.pitch),
            });
            NotePerformance::from_note(note)
                .map_err(|error| format!("工程校验失败：音符 {}：{error}", note.lyrics))?;
            PhonemeAttributes::read(&note.attributes)
                .map_err(|error| format!("工程校验失败：音符 {}：{error}", note.lyrics))?;
            TakeSet::from_extra(&note.extra)
                .map_err(|error| format!("工程校验失败：音符 {}：{error}", note.lyrics))?;
        }
        for (name, curve) in g.parameters.iter().chain(&g.vocal_modes) {
            if name.trim().is_empty() {
                return bad("参数名称不能为空");
            }
            curve
                .validate()
                .map_err(|error| format!("工程校验失败：参数 {name}：{error}"))?;
        }
        let mut control_bounds: Option<(i64, i64)> = None;
        for control in &g.pitch_controls {
            match control {
                PitchControl::Point(point) => {
                    if !point.valid() {
                        return bad("音高控制点的音高或标识无效");
                    }
                    control_bounds = Some(match control_bounds {
                        Some((min_t, max_t)) => (min_t.min(point.pos), max_t.max(point.pos)),
                        None => (point.pos, point.pos),
                    });
                }
                PitchControl::Curve(curve) => {
                    let Some((start, end)) = curve.valid().then(|| curve.range()).flatten() else {
                        return bad("手绘音高曲线必须包含至少两个按时间递增的有效控制点");
                    };
                    control_bounds = Some(match control_bounds {
                        Some((min_t, max_t)) => (min_t.min(start), max_t.max(end)),
                        None => (start, end),
                    });
                }
                PitchControl::Unsupported(_) => {}
            }
        }
        if groups
            .insert(
                g.uuid.as_str(),
                GroupBounds {
                    notes: note_bounds,
                    pitches: pitch_bounds,
                    pitch_controls: control_bounds,
                },
            )
            .is_some()
        {
            return bad("音符组 UUID 缺失或重复");
        }
    }
    for track in &p.tracks {
        if track.audio_clip().is_some_and(|c| !c.valid()) {
            return bad("伴奏文件路径或时间范围无效");
        }
        if track.is_instrumental()
            && (!track.groups.is_empty() || !track.main_group.notes.is_empty())
        {
            return bad("伴奏轨不能包含音符组");
        }
        if track.main_ref.group_id != track.main_group.uuid {
            return bad("主组引用不匹配");
        }
        for r in track.references() {
            TakeSet::from_extra(&r.extra)
                .map_err(|error| format!("工程校验失败：音轨 {}：{error}", track.name))?;
            let Some(bounds) = groups.get(r.group_id.as_str()) else {
                return bad("存在悬空音符组引用");
            };
            if r.blick_absolute_begin < 0
                || (r.blick_absolute_end != -1 && r.blick_absolute_end < r.blick_absolute_begin)
            {
                return bad("片段可见范围无效");
            }
            if let Some((min_o, max_e)) = bounds.notes
                && (min_o.checked_add(r.blick_offset).is_none()
                    || max_e.checked_add(r.blick_offset).is_none())
            {
                return bad("实例时间或音高溢出");
            }
            if let Some((min_p, max_p)) = bounds.pitches
                && (min_p.checked_add(r.pitch_offset).is_none()
                    || max_p.checked_add(r.pitch_offset).is_none())
            {
                return bad("实例时间或音高溢出");
            }
            if let Some((min_t, max_t)) = bounds.pitch_controls
                && (min_t.checked_add(r.blick_offset).is_none()
                    || max_t.checked_add(r.blick_offset).is_none())
            {
                return bad("实例音高控制时间溢出");
            }
        }
    }
    Ok(())
}

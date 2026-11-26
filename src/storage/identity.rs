use super::Revision;
use crate::model::Project;
use crate::{tr, trf};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// Derive reproducible identities for legacy records that omit them. Group
/// definition identities are validated because references need an unambiguous target.
pub(super) fn complete(project: &mut Project, revision: Revision) {
    let namespace = revision.hex();
    let generated = |kind: &str, first: usize, second: usize, attempt: usize| {
        let key = format!("rynthv:{namespace}:{kind}:{first}:{second}:{attempt}");
        let digest = Sha256::digest(key.as_bytes());
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&digest[..16]);
        uuid::Builder::from_custom_bytes(bytes).into_uuid()
    };
    if project.uuid.is_empty() {
        project.uuid = generated("project", 0, 0, 0).to_string();
    }
    for (index, group) in project.library.iter_mut().enumerate() {
        if group.uuid.is_empty() {
            group.uuid = generated("library", index, 0, 0).to_string();
        }
    }
    for (index, track) in project.tracks.iter_mut().enumerate() {
        if track.main_group.uuid.is_empty() {
            track.main_group.uuid = if track.main_ref.group_id.is_empty() {
                generated("main_group", index, 0, 0).to_string()
            } else {
                track.main_ref.group_id.clone()
            };
        }
        if track.main_ref.group_id.is_empty() {
            track.main_ref.group_id.clone_from(&track.main_group.uuid);
        }
    }
    let mut notes: HashSet<_> = project
        .library
        .iter()
        .chain(project.tracks.iter().map(|track| &track.main_group))
        .flat_map(|group| &group.notes)
        .filter_map(|note| u64::from_str_radix(&note.uuid, 16).ok())
        .collect();
    let mut encountered = HashSet::new();
    let groups = project
        .library
        .iter_mut()
        .chain(project.tracks.iter_mut().map(|track| &mut track.main_group));
    for (index, group) in groups.enumerate() {
        for (slot, note) in group.notes.iter_mut().enumerate() {
            let duplicate = note.uuid.len() == 16
                && u64::from_str_radix(&note.uuid, 16).is_ok_and(|id| !encountered.insert(id));
            if note.uuid.is_empty() || duplicate {
                let id = (0..)
                    .map(|attempt| generated("note", index, slot, attempt).as_u64_pair().0)
                    .find(|&id| notes.insert(id))
                    .expect("UUID space is available");
                note.uuid = format!("{id:016x}");
            }
        }
    }
    let mut reserved: HashSet<_> = project
        .library
        .iter()
        .chain(project.tracks.iter().map(|t| &t.main_group))
        .map(|group| group.uuid.as_str())
        .chain(std::iter::once(project.uuid.as_str()))
        .chain(
            project
                .tracks
                .iter()
                .flat_map(|t| t.references())
                .map(|r| r.uuid.as_str()),
        )
        .filter_map(|id| uuid::Uuid::parse_str(id).ok())
        .collect();
    let mut encountered: HashSet<_> = project
        .library
        .iter()
        .chain(project.tracks.iter().map(|t| &t.main_group))
        .map(|group| group.uuid.as_str())
        .chain(std::iter::once(project.uuid.as_str()))
        .filter_map(|id| uuid::Uuid::parse_str(id).ok())
        .collect();
    for (index, track) in project.tracks.iter_mut().enumerate() {
        for (slot, reference) in std::iter::once(&mut track.main_ref)
            .chain(&mut track.groups)
            .enumerate()
        {
            let duplicate =
                uuid::Uuid::parse_str(&reference.uuid).is_ok_and(|id| !encountered.insert(id));
            if reference.uuid.is_empty() || duplicate {
                reference.uuid = (0..)
                    .map(|attempt| generated("reference", index, slot, attempt))
                    .find(|&id| reserved.insert(id))
                    .expect("UUID space is available")
                    .to_string();
            }
        }
    }
}

pub(super) fn validate(project: &Project) -> Result<(), String> {
    let mut ids = HashSet::new();
    let mut note_ids = HashSet::new();
    let mut check = |value: &str, kind: &str| {
        let id =
            uuid::Uuid::parse_str(value).map_err(|_| trf!("工程校验失败：{} UUID 无效", kind))?;
        if !ids.insert(id) {
            return Err(trf!("工程校验失败：{} UUID 重复", kind));
        }
        Ok(())
    };
    check(&project.uuid, tr!("工程"))?;
    for group in project
        .library
        .iter()
        .chain(project.tracks.iter().map(|track| &track.main_group))
    {
        check(&group.uuid, tr!("音符组"))?;
        for note in &group.notes {
            if note.uuid.len() != 16 || !note.uuid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(tr!("工程校验失败：音符 UUID 必须为 16 位十六进制标识").into());
            }
            let id = u64::from_str_radix(&note.uuid, 16)
                .map_err(|_| tr!("工程校验失败：音符 UUID 无效"))?;
            if !note_ids.insert(id) {
                return Err(tr!("工程校验失败：音符 UUID 重复").into());
            }
        }
    }
    for track in &project.tracks {
        for reference in track.references() {
            check(&reference.uuid, tr!("音符组实例"))?;
        }
    }
    Ok(())
}

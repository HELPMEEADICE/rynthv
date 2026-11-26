use super::{VoiceAvailability, VoiceCatalog, VoiceCatalogEntry, VoiceFamily};
use crate::{model::singer_database, synthesis::settings::VoiceSelection, tr, trf};
use serde_json::Value;

impl VoiceCatalogEntry {
    pub fn selection(&self) -> VoiceSelection {
        VoiceSelection {
            database_id: self.database_id.clone(),
            revision: self.revision,
        }
    }
}

impl VoiceCatalog {
    pub fn entry(&self, selection: &VoiceSelection) -> Option<&VoiceCatalogEntry> {
        self.entries
            .iter()
            .find(|e| e.database_id == selection.database_id && e.revision == selection.revision)
    }

    pub fn resolve(
        &self,
        track: &Value,
        instance: &Value,
    ) -> Result<Option<VoiceSelection>, String> {
        let database = singer_database(track, instance)
            .as_object()
            .ok_or(tr!("歌声 database 必须是对象"))?;
        let text = |key| -> Result<&str, String> {
            match database.get(key) {
                None => Ok(""),
                Some(Value::String(s)) => Ok(s),
                _ => Err(trf!("歌声 database.{0} 必须是字符串", key)),
            }
        };
        let id = text("id")?;
        let name = text("name")?;
        if id.is_empty() && name.is_empty() {
            return Ok(None);
        }
        let version = text("version")?;
        let native_sv1 =
            id.is_empty() && database.get("backendType").and_then(Value::as_str) == Some("SVR2AI");
        let revision = if version.is_empty() || version == "-2" {
            None
        } else {
            Some(
                version
                    .parse::<u32>()
                    .map_err(|_| trf!("歌声版本无效：{0}", version))?,
            )
        };
        let matches: Vec<_> = self
            .entries
            .iter()
            .filter(|e| {
                (id.is_empty() || e.database_id == id)
                    && (name.is_empty() || e.display_name == name)
                    && (!native_sv1 || e.family == Some(VoiceFamily::Sv1))
                    && revision.is_none_or(|r| e.revision == r)
            })
            .collect();
        let entry = matches
            .iter()
            .copied()
            .max_by_key(|e| (e.availability.is_loadable(), e.revision))
            .ok_or_else(|| {
                trf!(
                    "找不到歌声 {0}（版本 {1}），请检查 databases 目录",
                    if name.is_empty() { id } else { name },
                    if version.is_empty() {
                        tr!("默认")
                    } else {
                        version
                    }
                )
            })?;
        if id.is_empty()
            && matches.iter().any(|other| {
                other.database_id != entry.database_id
                    && (!native_sv1
                        || other.vendor != entry.vendor
                        || (other.revision == entry.revision
                            && other.availability.is_loadable()
                                == entry.availability.is_loadable()))
            })
        {
            return Err(trf!("存在多个同名歌声 {0}，请为音轨重新选择歌手", name));
        }
        if let VoiceAvailability::Unsupported(reason) = &entry.availability {
            return Err(trf!("歌声 {0}（版本 {1}）不可用：{2}", entry.display_name, entry.revision, reason));
        }
        Ok(Some(entry.selection()))
    }
}

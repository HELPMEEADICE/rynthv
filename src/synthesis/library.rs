use super::{
    settings::SynthesisSettings,
    voice::{Result, VoiceAvailability, VoiceCatalog, flat::FlatModelRegistry, svpk},
};
use std::{path::Path, sync::Arc};

pub struct LibraryScan {
    pub catalog: VoiceCatalog,
    pub error: Option<String>,
    pub flat_registry: Arc<FlatModelRegistry>,
}

pub fn scan_roots(settings: &SynthesisSettings) -> LibraryScan {
    let flat_registry = Arc::new(settings.flat_registry());
    let mut catalog = VoiceCatalog::from_flat(&flat_registry);
    let mut errors = Vec::new();
    let mut append = |label: &str, root: &Path, result: Result<VoiceCatalog>| match result {
        Ok(found) => {
            catalog.entries.extend(found.entries);
            catalog.warnings.extend(
                found
                    .warnings
                    .into_iter()
                    .map(|warning| format!("{label} {}：{warning}", root.display())),
            );
        }
        Err(error) => errors.push(format!(
            "无法扫描 {label} 声库目录 {}：{error}",
            root.display()
        )),
    };
    for (label, root) in settings.database_roots() {
        append(label, root, VoiceCatalog::scan(root));
    }
    if let Ok(root) = svpk::root()
        && root.is_dir()
    {
        append("SVPK", &root, svpk::scan(&root));
    }
    catalog.entries.sort_by(|left, right| {
        left.database_id
            .cmp(&right.database_id)
            .then_with(|| left.revision.cmp(&right.revision))
            .then_with(|| left.directory.cmp(&right.directory))
    });
    catalog.entries.dedup_by(|left, right| {
        left.database_id == right.database_id
            && left.revision == right.revision
            && left.directory == right.directory
    });
    for duplicates in catalog.entries.chunk_by_mut(|left, right| {
        left.database_id == right.database_id && left.revision == right.revision
    }) {
        if duplicates.len() > 1 {
            let message = format!(
                "声库 {}/{} 对应多个文件，请检查声库目录配置。",
                duplicates[0].database_id, duplicates[0].revision
            );
            errors.push(message.clone());
            for entry in duplicates {
                entry.availability = VoiceAvailability::Unsupported(message.clone());
            }
        }
    }
    catalog.entries.sort_by(|left, right| {
        left.display_name
            .cmp(&right.display_name)
            .then_with(|| left.database_id.cmp(&right.database_id))
            .then_with(|| right.revision.cmp(&left.revision))
    });
    LibraryScan {
        catalog,
        error: (!errors.is_empty()).then(|| errors.join("\n")),
        flat_registry,
    }
}

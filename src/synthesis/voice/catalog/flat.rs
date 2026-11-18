use std::path::Path;

use serde::Serialize;

use super::{VoiceAvailability, VoiceCatalog, VoiceCatalogEntry};
use crate::synthesis::voice::{
    Result, VoiceFamily,
    config::VoiceConfiguration,
    flat::{
        FlatConditioning, FlatDependency, FlatModelRegistry, FlatPackage, ModelRole,
        is_managed_voice,
    },
};

#[derive(Clone, Debug, Serialize)]
pub struct FlatCatalogEntry {
    pub dependencies: Vec<FlatDependency>,
    pub styles: Vec<String>,
    pub managed: bool,
    pub format_version: Option<serde_json::Value>,
}

pub(super) fn catalog(registry: &FlatModelRegistry) -> VoiceCatalog {
    let mut catalog = VoiceCatalog {
        entries: Vec::new(),
        warnings: registry.warnings.clone(),
    };
    for (path, package) in &registry.packages {
        match entry(registry, path, package) {
            Ok(entry) => catalog.entries.push(entry),
            Err(error) => catalog
                .warnings
                .push(format!("{}: {error}", path.display())),
        }
    }
    catalog
}

fn entry(
    registry: &FlatModelRegistry,
    path: &Path,
    package: &FlatPackage,
) -> Result<VoiceCatalogEntry> {
    let manifest = &package.manifest;
    let dependencies = registry.dependencies(path, package)?;
    let missing = dependencies
        .iter()
        .filter(|dependency| dependency.is_missing())
        .map(FlatDependency::label)
        .collect::<Vec<_>>();
    let conditioning = FlatConditioning::from_manifest(manifest);
    let availability = if !missing.is_empty() {
        VoiceAvailability::Unsupported(format!("缺少 Flat 模型：{}", missing.join("、")))
    } else if dependencies
        .iter()
        .any(|dependency| dependency.role == ModelRole::SingingAssistant)
    {
        VoiceAvailability::Unsupported("此声库声明了暂不支持的 Flat sing_model".into())
    } else if let Err(error) = &conditioning {
        VoiceAvailability::Unsupported(error.to_string())
    } else {
        VoiceAvailability::Loadable
    };
    let styles = conditioning.map_or_else(
        |_| {
            manifest
                .styles
                .iter()
                .map(|style| style.name.clone())
                .collect()
        },
        |(_, styles)| styles,
    );
    let config = VoiceConfiguration::from_flat(manifest, styles.clone());
    let managed = is_managed_voice(path);
    let icon_path = managed
        .then(|| path.with_extension("png"))
        .filter(|icon| icon.is_file());
    Ok(VoiceCatalogEntry {
        database_id: manifest.database_id(),
        revision: manifest.revision()?,
        model_version: Some(1),
        family: Some(VoiceFamily::Sv1),
        directory: path.to_owned(),
        display_name: config.name,
        vendor: config.vendor,
        genres: Vec::new(),
        pitch_range: None,
        languages: config.supported_languages,
        icon_path,
        flat: Some(FlatCatalogEntry {
            dependencies,
            styles,
            managed,
            format_version: manifest.format_version.clone(),
        }),
        availability,
    })
}

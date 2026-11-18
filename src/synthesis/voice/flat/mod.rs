//! SynthV Flat package inventory. Archive paths are never extracted as-is.

mod archive;
mod conditioning;
mod embedding;
mod library;
mod manifest;
mod phones;
mod registry;
mod resolver;

pub use archive::{FlatArchive, PackageKind};
pub(crate) use conditioning::FlatConditioning;
pub use library::{ImportedFlatVoice, import, remove_import};
pub use manifest::{FlatManifest, FlatStyle, ModelRole, PackageConfig};
pub use registry::{FlatModelRegistry, ModelSource};
pub use resolver::{FlatDependency, FlatModel, ResolvedFlatModels};

use std::path::Path;

use serde::Serialize;

use super::Result;

pub fn is_voice_path(path: &Path) -> bool {
    archive::is_sfpk(path)
}

pub(super) fn is_managed_voice(path: &Path) -> bool {
    path.ancestors()
        .nth(3)
        .is_some_and(|root| library::is_managed(&library::imports_root(root), path))
}

#[derive(Debug, Serialize)]
pub struct FlatPackage {
    pub manifest: FlatManifest,
    pub config: Option<PackageConfig>,
    pub resources: Vec<String>,
}

impl FlatPackage {
    pub fn inspect(path: &Path) -> Result<Vec<Self>> {
        let mut archive = FlatArchive::open(path)?;
        archive.visit_packages(|_, package| Ok(package))
    }
}

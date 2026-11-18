//! SynthV Flat package inventory. Archive paths are never extracted as-is.

mod archive;
mod conditioning;
mod embedding;
mod manifest;
mod registry;
mod resolver;

pub use archive::{FlatArchive, PackageKind};
pub(crate) use conditioning::FlatConditioning;
pub use manifest::{FlatManifest, FlatStyle, ModelRole, PackageConfig};
pub use registry::{FlatModelRegistry, ModelSource};
pub use resolver::{FlatDependency, FlatModel, ResolvedFlatModels};

use std::path::Path;

use serde::Serialize;

use super::Result;

pub fn is_voice_path(path: &Path) -> bool {
    archive::is_sfpk(path)
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

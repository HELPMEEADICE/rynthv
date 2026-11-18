//! SynthV Flat package inventory. Archive paths are never extracted as-is.

mod archive;
mod embedding;
mod manifest;

pub use archive::{FlatArchive, PackageKind};
pub use manifest::{FlatManifest, FlatStyle, ModelRole, PackageConfig};

use std::path::Path;

use serde::Serialize;

use super::Result;

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

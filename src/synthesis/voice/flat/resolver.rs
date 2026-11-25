use crate::platform::fs::File;
use std::{
    collections::HashMap,
    io::Read,
    path::Path,
    sync::{Arc, Mutex, OnceLock, Weak},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{
    FlatArchive, FlatModelRegistry, FlatPackage, ModelRole, ModelSource, registry::model_member,
};
use crate::synthesis::voice::{PhoneSet, PreparedTree, Result, VoiceError, embedded, hex};

const MAX_MODEL_BYTES: usize = 256 * 1024 * 1024;
const REQUIRED: [ModelRole; 4] = [
    ModelRole::Acoustic,
    ModelRole::Vocoder,
    ModelRole::Duration,
    ModelRole::F0,
];

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct FlatDependency {
    pub role: ModelRole,
    pub id: Option<String>,
    pub sources: Vec<ModelSource>,
}

#[derive(Serialize)]
pub struct FlatModel {
    pub role: ModelRole,
    pub id: String,
    pub sha256: String,
    pub bytes: usize,
    #[serde(skip)]
    pub tree: Arc<PreparedTree>,
}

pub struct ResolvedFlatModels {
    pub package: FlatPackage,
    pub models: Vec<FlatModel>,
    pub phones: Vec<PhoneSet>,
}

impl FlatModelRegistry {
    pub fn dependencies(&self, path: &Path, package: &FlatPackage) -> Result<Vec<FlatDependency>> {
        let ids = package.manifest.model_ids()?;
        let mut dependencies = Vec::new();
        for role in REQUIRED.into_iter().chain(
            ids.contains_key(&ModelRole::SingingAssistant)
                .then_some(ModelRole::SingingAssistant),
        ) {
            let id = ids.get(&role).copied();
            let sources = id.map_or_else(Vec::new, |id| {
                let member = model_member(id);
                if package.resources.binary_search(&member).is_ok() {
                    return vec![ModelSource::Package {
                        path: path.to_owned(),
                        member,
                    }];
                }
                self.candidates(role, id).into_iter().collect()
            });
            dependencies.push(FlatDependency {
                role,
                id: id.map(str::to_owned),
                sources,
            });
        }
        Ok(dependencies)
    }

    pub fn resolve(&self, path: &Path) -> Result<ResolvedFlatModels> {
        let mut archive = FlatArchive::open(path)?;
        let package = archive.describe()?;
        let dependencies = self.dependencies(path, &package)?;
        // Report all missing dependencies before reading any weight payload.
        let missing = dependencies
            .iter()
            .filter(|dependency| dependency.is_missing())
            .map(FlatDependency::label)
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(VoiceError::new(format!(
                "Missing Flat model dependencies: {}",
                missing.join(", ")
            )));
        }
        if let Some(assistant) = dependencies
            .iter()
            .find(|model| model.role == ModelRole::SingingAssistant)
        {
            return Err(VoiceError::new(format!(
                "Unsupported Flat singing assistant model {}",
                assistant.label()
            )));
        }
        let phones = super::phones::read(&mut archive, &package)?;
        let models = dependencies
            .into_iter()
            .map(|dependency| load_dependency(path, &mut archive, dependency))
            .collect::<Result<Vec<_>>>()?;
        Ok(ResolvedFlatModels {
            package,
            models,
            phones,
        })
    }
}

impl FlatDependency {
    pub fn is_missing(&self) -> bool {
        self.sources.is_empty()
    }

    pub fn label(&self) -> String {
        self.id.as_ref().map_or_else(
            || format!("{} (model ID absent)", self.role.directory()),
            |id| format!("{}/{id}", self.role.directory()),
        )
    }
}

impl ModelRole {
    pub fn stage(self) -> &'static str {
        match self {
            Self::Acoustic => "timbre",
            Self::SingingAssistant => "sing",
            _ => self.directory(),
        }
    }

    fn accepts(self, root: [u8; 16]) -> bool {
        use embedded::types;
        match self {
            Self::Acoustic => root == types::TIMBRE_TYPE_ID,
            Self::Vocoder => root == types::VOCODER_TYPE_ID,
            Self::Duration => root == types::DURATION_TYPE_ID,
            Self::F0 => root == types::F0_WRAPPER_TYPE_ID,
            Self::SingingAssistant => false,
        }
    }
}

fn load_dependency(
    path: &Path,
    archive: &mut FlatArchive,
    dependency: FlatDependency,
) -> Result<FlatModel> {
    let role = dependency.role;
    let id = dependency
        .id
        .expect("presence checked before loading models");
    let load = || -> Result<_> {
        let mut first = None;
        for source in dependency.sources {
            let bytes = match source {
                ModelSource::Package {
                    path: source,
                    member,
                } if source == path => archive.read(&member, MAX_MODEL_BYTES)?,
                ModelSource::Package { path, member } => {
                    FlatArchive::open(&path)?.read(&member, MAX_MODEL_BYTES)?
                }
                ModelSource::File { path } => read_model(&path)?,
            };
            let digest: [u8; 32] = Sha256::digest(&bytes).into();
            if let Some((_, previous)) = &first {
                if *previous != digest {
                    return Err(VoiceError::new(
                        "Conflicting registered sources have different model contents",
                    ));
                }
            } else {
                first = Some((bytes, digest));
            }
        }
        let (bytes, digest) = first.expect("sources checked before loading models");
        let root = embedded::root_type(&bytes)?;
        if !role.accepts(root) {
            return Err(VoiceError::new(format!(
                "Incompatible embedded DNNI root {}",
                hex(&root)
            )));
        }
        Ok(FlatModel {
            role,
            id: id.clone(),
            sha256: hex(&digest),
            bytes: bytes.len(),
            tree: shared_tree(digest, &bytes)?,
        })
    };
    load().map_err(|error| VoiceError::new(format!("Flat {}/{id}: {error}", role.directory())))
}

fn read_model(path: &Path) -> Result<Vec<u8>> {
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_MODEL_BYTES as u64 {
        return Err(VoiceError::new(format!(
            "Invalid Flat model file size or type: {}",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    file.take(MAX_MODEL_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != metadata.len() {
        return Err(VoiceError::new(format!(
            "Flat model changed while reading {}",
            path.display()
        )));
    }
    Ok(bytes)
}

type TreeSlot = Mutex<Weak<PreparedTree>>;

fn shared_tree(digest: [u8; 32], bytes: &[u8]) -> Result<Arc<PreparedTree>> {
    static CACHE: OnceLock<Mutex<HashMap<[u8; 32], Arc<TreeSlot>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    let slot = {
        let mut slots = cache
            .lock()
            .map_err(|_| VoiceError::new("Flat model cache is poisoned"))?;
        slots.retain(|_, slot| {
            Arc::strong_count(slot) > 1 || slot.lock().is_ok_and(|tree| tree.strong_count() > 0)
        });
        Arc::clone(slots.entry(digest).or_default())
    };
    let mut tree = slot
        .lock()
        .map_err(|_| VoiceError::new("Flat model cache slot is poisoned"))?;
    if let Some(cached) = tree.upgrade() {
        return Ok(cached);
    }
    let prepared = Arc::new(PreparedTree::prepare_flat_embedded(bytes)?);
    *tree = Arc::downgrade(&prepared);
    Ok(prepared)
}

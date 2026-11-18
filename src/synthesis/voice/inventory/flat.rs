use std::{collections::BTreeMap, path::Path, sync::{Arc, OnceLock}};

use sha2::{Digest, Sha256};

use super::{GraphSummary, TensorSummary, VoiceFile, VoiceInventory};
use crate::synthesis::voice::{
    PreparedModule, Result, VoiceError, config::VoiceConfiguration,
    flat::{FlatConditioning, FlatModel, FlatModelRegistry}, hex,
    preparation::{PreparedFile, PreparedModel, PreparedVoice},
};

pub(super) fn load(path: &Path, registry: &FlatModelRegistry) -> Result<VoiceInventory> {
    let resolved = registry.resolve(path)?;
    let manifest = resolved.package.manifest;
    let (conditioning, styles) = FlatConditioning::from_manifest(&manifest)?;
    let configuration = VoiceConfiguration::from_flat(&manifest, styles);
    let mut digest = Sha256::new();
    digest.update(b"RynthV Flat voice manifest and DDS models\0");
    digest.update(serde_json::to_vec(&manifest).map_err(|error| VoiceError::new(error.to_string()))?);
    let mut files = Vec::new();
    let mut prepared_files = Vec::new();
    for model in resolved.models {
        digest.update(model.role.stage().as_bytes());
        digest.update(model.sha256.as_bytes());
        let file = describe(&model)?;
        prepared_files.push(PreparedFile {
            name: file.name.clone(),
            trees: vec![PreparedModel { role: model.role.stage(), tree: model.tree }],
        });
        files.push(file);
    }
    let prepared = Arc::new(PreparedVoice::new(
        prepared_files,
        configuration.family,
        configuration.f0_enhancement,
        configuration.styles.clone(),
        Some(conditioning),
    ));
    if let Some(error) = &prepared.summary().stage_preparation_error {
        return Err(VoiceError::new(format!("Incompatible Flat native stages: {error}")));
    }
    Ok(VoiceInventory {
        database_id: manifest.database_id(),
        directory: path.to_owned(),
        display_name: configuration.name.clone(),
        content_hash: hex(&digest.finalize()),
        resource_count: files.iter().map(|file| file.resources).sum(),
        graph_count: files.len(),
        tensor_count: files.iter().flat_map(|file| file.tensors.values()).map(|tensor| tensor.resources).sum(),
        files,
        configuration: Some(configuration),
        flat: Some(manifest),
        _models: Vec::new(),
        preparation: OnceLock::from(Ok(prepared)),
    })
}

fn describe(model: &FlatModel) -> Result<VoiceFile> {
    let tree = &model.tree;
    let root = tree.node(tree.root_index())?;
    let mut depths = Vec::<usize>::with_capacity(tree.nodes().len());
    let mut tensors = BTreeMap::<_, TensorSummary>::new();
    for node in tree.nodes() {
        let depth = node.children.iter().map(|&child| depths[child] + 1).max().unwrap_or(0);
        depths.push(depth);
        if let PreparedModule::Tensor(tensor) = &node.module {
            let layout = tensor.layout();
            let stats = tensors.entry(layout.storage).or_default();
            stats.resources += 1;
            stats.elements += layout.rows * layout.columns;
            stats.decoded_resources += 1;
        }
    }
    let resources = tree.nodes().len();
    Ok(VoiceFile {
        name: format!("{}/{}.dnni", model.role.directory(), model.id),
        bytes: model.bytes,
        sha256: model.sha256.clone(),
        sha256_scope: "expanded_flat_model",
        resources,
        borrowed_resources: 0,
        encoded_resources: 0,
        header_signature_equal_branch_verified: false,
        index_digest_verified: false,
        all_resource_digests_verified: false,
        graphs: vec![GraphSummary {
            resource_index: root.resource_index,
            nodes: resources,
            root_children: root.children.len(),
            maximum_depth: depths[tree.root_index()],
            role: model.role.stage(),
        }],
        referenced_modules: resources,
        unreferenced_resources: 0,
        tensors,
        configuration_parsed: false,
    })
}

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, OnceLock},
};

use sha2::{Digest, Sha256};

use super::{GraphSummary, TensorSummary, VoiceFile, VoiceInventory};
use crate::synthesis::voice::{
    PreparedModule, Result, VoiceError,
    config::VoiceConfiguration,
    flat::{FlatConditioning, FlatModel, FlatModelRegistry, ModelRole},
    hex,
    preparation::{PreparedFile, PreparedModel, PreparedVoice},
};

pub(super) fn load(path: &Path, registry: &FlatModelRegistry) -> Result<VoiceInventory> {
    let resolved = registry.resolve(path)?;
    let manifest = resolved.package.manifest;
    let (mut conditioning, styles) = FlatConditioning::from_manifest(&manifest)?;
    conditioning.phones = resolved.phones;
    let mut configuration = VoiceConfiguration::from_flat(&manifest, styles);
    let excluded = filter_languages(&mut configuration, &resolved.models)?;
    let flat_warnings = excluded
        .into_iter()
        .map(|language| {
            format!(
                "Flat language {language:?} is missing from a native DDS stage or lyric frontend"
            )
        })
        .collect();
    let mut digest = Sha256::new();
    digest.update(b"RynthV Flat voice manifest, phone tables and DDS models\0");
    digest
        .update(serde_json::to_vec(&manifest).map_err(|error| VoiceError::new(error.to_string()))?);
    digest.update(
        serde_json::to_vec(&conditioning.phones)
            .map_err(|error| VoiceError::new(error.to_string()))?,
    );
    let mut files = Vec::new();
    let mut prepared_files = Vec::new();
    for model in resolved.models {
        digest.update(model.role.stage().as_bytes());
        digest.update(model.sha256.as_bytes());
        let file = describe(&model)?;
        prepared_files.push(PreparedFile {
            name: file.name.clone(),
            trees: vec![PreparedModel {
                role: model.role.stage(),
                tree: model.tree,
            }],
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
        return Err(VoiceError::new(format!(
            "Incompatible Flat native stages: {error}"
        )));
    }
    Ok(VoiceInventory {
        database_id: manifest.database_id(),
        directory: path.to_owned(),
        display_name: configuration.name.clone(),
        content_hash: hex(&digest.finalize()),
        resource_count: files.iter().map(|file| file.resources).sum(),
        graph_count: files.len(),
        tensor_count: files
            .iter()
            .flat_map(|file| file.tensors.values())
            .map(|tensor| tensor.resources)
            .sum(),
        files,
        configuration: Some(configuration),
        flat: Some(manifest),
        flat_warnings,
        _models: Vec::new(),
        preparation: OnceLock::from(Ok(prepared)),
    })
}

fn filter_languages(config: &mut VoiceConfiguration, models: &[FlatModel]) -> Result<Vec<String>> {
    let mut excluded = Vec::new();
    config.supported_languages.retain(|language| {
        let supported = crate::model::Language::parse(language).is_some()
            && models
                .iter()
                .filter(|model| {
                    matches!(
                        model.role,
                        ModelRole::Acoustic | ModelRole::Duration | ModelRole::F0
                    )
                })
                .all(|model| {
                    model.tree.nodes().iter().any(|node| {
                        matches!(&node.module, PreparedModule::PhoneSet(table)
                        if table.name.split_once('-').is_some_and(|(prefix, _)| prefix == language))
                    })
                });
        if !supported {
            excluded.push(language.clone());
        }
        supported
    });
    if excluded.contains(&config.default_language) {
        return Err(VoiceError::new(format!(
            "Default Flat language {:?} is missing from a native DDS stage or lyric frontend",
            config.default_language
        )));
    }
    Ok(excluded)
}

fn describe(model: &FlatModel) -> Result<VoiceFile> {
    let tree = &model.tree;
    let root = tree.node(tree.root_index())?;
    let mut depths = Vec::<usize>::with_capacity(tree.nodes().len());
    let mut tensors = BTreeMap::<_, TensorSummary>::new();
    for node in tree.nodes() {
        let depth = node
            .children
            .iter()
            .map(|&child| depths[child] + 1)
            .max()
            .unwrap_or(0);
        depths.push(depth);
        if let PreparedModule::Tensor(tensor) = &node.module {
            let layout = tensor.layout();
            let stats = tensors.entry(layout.storage).or_default();
            stats.resources += 1;
            stats.elements += layout.elements;
            stats.scales += layout.scales;
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

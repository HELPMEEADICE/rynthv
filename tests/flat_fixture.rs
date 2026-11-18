//! Synthetic format fixtures. The DNNI records contain topology markers only.

use std::{
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use rynthv::synthesis::voice::{
    VoiceCatalog, VoiceInventory,
    flat::{
        self, FlatArchive, FlatManifest, FlatModelRegistry, FlatPackage, ModelRole, PackageKind,
    },
};
use serde_json::{Value, json};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

#[test]
fn synthetic_magic_and_manifest_validation() {
    let files = Scratch::new();
    let payload = manifest("Synthetic voice");
    let named_as_batch = files.write("test.sfpks", &zip(&[("info.nofs", &payload)]));
    let mut archive = FlatArchive::open(&named_as_batch).unwrap();
    assert_eq!(archive.kind(), PackageKind::Sfpk);
    let package = archive.describe().unwrap();
    assert_eq!(
        package.manifest.model_ids().unwrap()[&ModelRole::Acoustic],
        "base_model_fixture"
    );
    assert_eq!(
        package.manifest.format_version,
        Some(json!("1.4.x-fixture"))
    );
    assert_eq!(
        package.manifest.additional["fixture_future_field"],
        json!({"enabled": true})
    );
    assert_eq!(package.manifest.styles[1].extra_value().unwrap(), Some(0.5));
    assert_eq!(
        package.manifest.pitch_embedding().unwrap().unwrap(),
        [0.0; 32]
    );
    assert_eq!(
        package.manifest.timing_embedding().unwrap().unwrap(),
        [0.0; 128]
    );

    let wrong_magic = files.write("invalid.sfpk", b"SFPK invalid");
    assert!(FlatArchive::open(&wrong_magic).is_err());
    let not_a_voice = files.write("empty.sfpk", &zip(&[("unrelated.json", b"{}")]));
    assert!(FlatArchive::open(&not_a_voice).is_err());
    let oversized = files.write(
        "oversized.sfpk",
        &zip(&[("info.nofs", &vec![b' '; 1024 * 1024 + 1])]),
    );
    assert!(
        FlatPackage::inspect(&oversized)
            .unwrap_err()
            .to_string()
            .contains("size limit")
    );

    let mut corrupt = zip(&[("info.nofs", &payload)]);
    let offset = {
        let mut raw = ZipArchive::new(Cursor::new(&corrupt)).unwrap();
        raw.by_name("info.nofs").unwrap().data_start().unwrap() as usize
    };
    corrupt[offset] ^= 1;
    assert!(FlatPackage::inspect(&files.write("corrupt.sfpk", &corrupt)).is_err());

    let mut manifest_json: Value = serde_json::from_slice(&payload).unwrap();
    manifest_json["styles"][0]["data"] = json!("0000807f".repeat(32));
    assert!(FlatManifest::parse(&serde_json::to_vec(&manifest_json).unwrap()).is_err());
    manifest_json["styles"][0]["data"] = json!("00".repeat(128));
    manifest_json["acoustic"] = json!("conflicting");
    assert!(FlatManifest::parse(&serde_json::to_vec(&manifest_json).unwrap()).is_err());
}

#[test]
fn synthetic_batch_import_is_atomic_and_removes_only_managed_copies() {
    let files = Scratch::new();
    let a = zip(&[("info.nofs", &manifest("Fixture A"))]);
    let b = zip(&[("voice.nofs", &manifest("Fixture B"))]);
    let inputs = files.write(
        "fixtures.sfpks",
        &zip(&[("folder/a.sfpk", &a), ("b.sfpk", &b)]),
    );
    assert_eq!(
        FlatArchive::open(&inputs).unwrap().kind(),
        PackageKind::Sfpks
    );
    assert_eq!(FlatPackage::inspect(&inputs).unwrap().len(), 2);
    let broken = files.write("broken.sfpks", &zip(&[("a.sfpk", &a), ("b.sfpk", b"bad")]));
    let root = files.path.join("library");
    assert!(flat::import(&broken, &root).is_err());
    assert!(
        FlatModelRegistry::scan(Some(&root), None)
            .packages
            .is_empty()
    );
    let imported = flat::import(&inputs, &root).unwrap();
    assert_eq!(imported.len(), 2);
    assert!(flat::import(&inputs, &root).is_err());
    let registry = FlatModelRegistry::scan(Some(&root), None);
    let catalog = VoiceCatalog::from_flat(&registry);
    assert_eq!(catalog.entries.len(), 2);
    assert!(
        catalog
            .entries
            .iter()
            .all(|entry| !entry.availability.is_loadable())
    );
    flat::remove_import(&root, &imported[0].path).unwrap();
    assert!(inputs.is_file());
    assert!(imported[1].path.is_file());
    assert_eq!(FlatModelRegistry::scan(Some(&root), None).packages.len(), 1);
    assert!(flat::remove_import(&root, &inputs).is_err());
}

#[test]
fn synthetic_registry_distinguishes_missing_conflicting_and_shared_models() {
    let files = Scratch::new();
    let donor = files.write("donor.sfpk", &synthetic_model_package("Donor"));
    let bare = files.write("bare.sfpk", &zip(&[("info.nofs", &manifest("Thin voice"))]));
    let unavailable = FlatModelRegistry::default();
    let Err(error) = unavailable.resolve(&bare) else {
        panic!("resolved missing weights")
    };
    assert!(
        error
            .to_string()
            .contains("Missing Flat model dependencies")
    );

    let registry = FlatModelRegistry::scan(Some(&files.path), None);
    let full = registry.resolve(&donor).unwrap();
    let shared = registry.resolve(&bare).unwrap();
    assert_eq!(full.models.len(), 4);
    for (left, right) in full.models.iter().zip(&shared.models) {
        assert_eq!(left.sha256, right.sha256);
        assert!(Arc::ptr_eq(&left.tree, &right.tree));
    }
    // Canonical root tags alone are insufficient for native synthesis.
    assert!(VoiceInventory::load_flat(&donor, &registry).is_err());

    let external = files.path.join("models");
    fs::create_dir_all(external.join("acoustic")).unwrap();
    let duplicate = external.join("acoustic/base_model_fixture.dnni");
    fs::write(&duplicate, root(0x65f83b630b2ee1b8)).unwrap();
    let registry = FlatModelRegistry::scan(Some(&files.path), Some(&external));
    assert!(registry.resolve(&bare).is_ok());
    fs::write(&duplicate, root(0x4c30eaff390f599a)).unwrap();
    let Err(error) = registry.resolve(&bare) else {
        panic!("resolved conflicting weights")
    };
    assert!(error.to_string().contains("Conflicting registered sources"));

    let invalid = files.write(
        "invalid.sfpk",
        &zip(&[
            ("info.nofs", &manifest("Wrong acoustic role")),
            ("models/base_model_fixture.dnni", &root(0x4c30eaff390f599a)),
        ]),
    );
    let Err(error) = registry.resolve(&invalid) else {
        panic!("accepted duration as acoustic")
    };
    assert!(
        error
            .to_string()
            .contains("Incompatible embedded DNNI root")
    );
}

#[test]
fn synthetic_cli_and_catalog_report_the_same_dependency_state() {
    let files = Scratch::new();
    let package = files.write(
        "voice.sfpk",
        &zip(&[("info.nofs", &manifest("CLI fixture"))]),
    );
    let registry = FlatModelRegistry::scan(Some(&files.path), None);
    let catalog = VoiceCatalog::from_flat(&registry);
    let entry = catalog.entries.first().unwrap();
    let dependencies = serde_json::to_value(&entry.flat.as_ref().unwrap().dependencies).unwrap();
    let run = |command| {
        Command::new(env!("CARGO_BIN_EXE_rynthv-synth"))
            .arg(command)
            .arg("--input")
            .arg(&package)
            .arg("--flat-dir")
            .arg(&files.path)
            .arg("--model-dir")
            .arg(&files.path)
            .output()
            .unwrap()
    };
    let inspect = run("inspect-flat");
    assert!(inspect.status.success());
    let json: Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(json["dependencies"], dependencies);
    let validation = run("validate-flat");
    assert_eq!(validation.status.code(), Some(1));
    let json: Value = serde_json::from_slice(&validation.stdout).unwrap();
    assert_eq!(json["ok"], false);
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .contains("Missing Flat model dependencies")
    );
}

fn manifest(name: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "name": name, "version": "7", "vendor": "Fixture author", "language": "japanese",
        "base_model": "base_model_fixture", "vocoder": "vocoder_fixture",
        "timing_model": "duration_fixture", "f0_model": "f0_fixture",
        "pitch": "00".repeat(128), "timing": "00".repeat(512),
        "styles": [
            {"name": "(base)", "data": "00".repeat(128)},
            {"name": "Synthetic mode", "data": "00".repeat(128), "extra": "0000003f"}
        ],
        "format_version": "1.4.x-fixture",
        "fixture_future_field": {"enabled": true}
    }))
    .unwrap()
}

fn synthetic_model_package(name: &str) -> Vec<u8> {
    zip(&[
        ("info.nofs", &manifest(name)),
        ("models/base_model_fixture.dnni", &root(0x65f83b630b2ee1b8)),
        ("models/vocoder_fixture.dnni", &root(0xb244cc8626a05cae)),
        ("models/duration_fixture.dnni", &root(0x4c30eaff390f599a)),
        ("models/f0_fixture.dnni", &root(0xa81b4e0a342431ed)),
    ])
}

fn root(alias: u64) -> Vec<u8> {
    let child_count = alias as u32 ^ (alias >> 32) as u32 ^ 0xac5e7bd5;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0x7fca00ff_u32.to_le_bytes());
    bytes.extend_from_slice(&2u32.to_le_bytes());
    bytes.extend_from_slice(&0x7fca41ff_u32.to_le_bytes());
    bytes.extend_from_slice(&alias.to_le_bytes());
    bytes.extend_from_slice(&child_count.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes
}

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    for &(name, content) in entries {
        archive
            .start_file(
                name,
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )
            .unwrap();
        archive.write_all(content).unwrap();
    }
    archive.finish().unwrap().into_inner()
}

struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("rynthv-flat-fixture-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self { path }
    }

    fn write(&self, name: impl AsRef<Path>, bytes: &[u8]) -> PathBuf {
        let path = self.path.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

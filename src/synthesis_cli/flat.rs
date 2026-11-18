use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::ExitCode,
};

use rynthv::synthesis::{
    settings::SynthesisSettings,
    voice::{
        VoiceError, VoiceInventory,
        flat::{self, FlatArchive, FlatModelRegistry, FlatPackage, PackageKind},
    },
};
use serde_json::{Value, json};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Command {
    Inspect,
    Validate,
    Import,
    List,
}

impl Command {
    pub fn parse(name: &OsStr) -> Option<Self> {
        match name.to_str()? {
            "inspect-flat" => Some(Self::Inspect),
            "validate-flat" => Some(Self::Validate),
            "import-flat" => Some(Self::Import),
            "list-flat-models" => Some(Self::List),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Options {
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    models: Option<PathBuf>,
    voices: Option<PathBuf>,
}

pub(super) fn run(command: Command, args: impl Iterator<Item = OsString>) -> ExitCode {
    let options = match parse_options(command, args) {
        Ok(options) => options,
        Err(error) => return output(None, &json!({"ok": false, "error": error}), 2),
    };
    match execute(command, &options) {
        Ok(result) => output(options.output, &result, 0),
        Err(error) => output(options.output, &json!({"ok": false, "error": error}), 1),
    }
}

fn execute(command: Command, options: &Options) -> Result<Value, String> {
    let path = options.input.as_deref();
    let error = |error: VoiceError| error.to_string();
    if command == Command::Import {
        let directory = match &options.voices {
            Some(directory) => directory.clone(),
            None => SynthesisSettings::load()?.flat_voice_directory()?,
        };
        let imported =
            flat::import(path.expect("import input checked"), &directory).map_err(error)?;
        return Ok(json!({"ok": true, "imported": imported}));
    }
    let registry = registry(options.voices.as_deref(), options.models.as_deref(), path)?;
    match command {
        Command::List => Ok(json!({
            "ok": true,
            "models": registry.models().collect::<Vec<_>>(),
            "warnings": registry.warnings,
        })),
        Command::Inspect => {
            let path = path.expect("inspection input checked");
            let mut archive = FlatArchive::open(path).map_err(error)?;
            if archive.kind() == PackageKind::Sfpks {
                let packages = FlatPackage::inspect(path).map_err(error)?;
                return Ok(json!({"ok": true, "kind": "sfpks", "packages": packages}));
            }
            let package = archive.describe().map_err(error)?;
            let dependencies = registry.dependencies(path, &package).map_err(error)?;
            Ok(json!({
                "ok": true, "kind": "sfpk", "package": package,
                "database_id": package.manifest.database_id(), "revision": package.manifest.revision().map_err(error)?,
                "dependencies": dependencies, "warnings": registry.warnings,
            }))
        }
        Command::Validate => {
            let path = path.expect("validation input checked");
            let archive = FlatArchive::open(path).map_err(error)?;
            if archive.kind() != PackageKind::Sfpk {
                return Err("Import the batch and validate individual SFPK voices".into());
            }
            let inventory = VoiceInventory::load_flat(path, &registry).map_err(error)?;
            Ok(json!({
                "ok": true, "stage_programs_available": inventory.prepare().map_err(error)?.summary().stage_programs_available,
                "database_id": inventory.database_id, "sha256": inventory.content_hash,
                "configuration": inventory.configuration, "files": inventory.files,
                "warnings": registry.warnings.iter().chain(&inventory.flat_warnings).collect::<Vec<_>>(),
            }))
        }
        Command::Import => unreachable!("import handled separately"),
    }
}

pub(super) fn registry(
    voices: Option<&Path>,
    models: Option<&Path>,
    input: Option<&Path>,
) -> Result<FlatModelRegistry, String> {
    let mut registry = if let (Some(voices), Some(models)) = (voices, models) {
        FlatModelRegistry::scan(Some(voices), Some(models))
    } else {
        let settings = SynthesisSettings::load()?;
        let configured = settings.flat_voice_directory()?;
        let default = (settings.flat_databases.is_some() || configured.is_dir())
            .then_some(configured.as_path());
        FlatModelRegistry::scan(
            voices.or(default),
            models.or(settings.flat_models.as_deref()),
        )
    };
    if let Some(input) = input {
        include_neighbors(&mut registry, input);
    }
    Ok(registry)
}

pub(super) fn include_neighbors(registry: &mut FlatModelRegistry, path: &Path) {
    if let Some(parent) = path.parent() {
        registry.extend(FlatModelRegistry::scan(Some(parent), None));
    }
}

fn parse_options(
    command: Command,
    mut args: impl Iterator<Item = OsString>,
) -> Result<Options, String> {
    let mut options = Options::default();
    while let Some(flag) = args.next() {
        let slot = match flag.to_str() {
            Some("--input") if command != Command::List => &mut options.input,
            Some("--output") => &mut options.output,
            Some("--model-dir") if command != Command::Import => &mut options.models,
            Some("--flat-dir") => &mut options.voices,
            _ => return Err(format!("Unknown option {}", flag.to_string_lossy())),
        };
        if slot.is_some() {
            return Err(format!("Repeated option {}", flag.to_string_lossy()));
        }
        *slot = Some(PathBuf::from(
            args.next()
                .filter(|arg| !arg.to_string_lossy().starts_with("--"))
                .ok_or_else(|| format!("Missing value for {}", flag.to_string_lossy()))?,
        ));
    }
    if command != Command::List && options.input.is_none() {
        return Err("This Flat command requires --input PATH".into());
    }
    Ok(options)
}

fn output(path: Option<PathBuf>, report: &Value, code: u8) -> ExitCode {
    match super::write_report(path, report) {
        Ok(()) => ExitCode::from(code),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

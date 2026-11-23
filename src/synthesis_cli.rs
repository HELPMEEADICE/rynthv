//! Headless resource preparation and individual model-module execution.
#[path = "synthesis_cli/args.rs"]
mod args;
#[path = "synthesis_cli/context.rs"]
mod context;
#[path = "synthesis_cli/flat.rs"]
mod flat;
#[path = "synthesis_cli/render.rs"]
mod render;

use args::{Args, Command};
use rynthv::{
    storage,
    synthesis::{inference::compute, plan::Planner, voice::VoiceInventory},
};
use serde_json::json;
use std::{
    fs,
    io::{self, Write},
    process::ExitCode,
    time::Instant,
};

use std::io::Read;

struct CliError {
    usage: bool,
    message: String,
}

impl CliError {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            usage: true,
            message: message.into(),
        }
    }
}

impl From<String> for CliError {
    fn from(message: String) -> Self {
        Self {
            usage: false,
            message,
        }
    }
}

impl From<&str> for CliError {
    fn from(message: &str) -> Self {
        Self::from(message.to_owned())
    }
}

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    let Some(command) = arguments.next() else {
        print!("{}", args::HELP);
        return ExitCode::SUCCESS;
    };
    if let Some(command) = flat::Command::parse(&command) {
        return flat::run(command, arguments);
    }
    match run(command, arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", error.message);
            ExitCode::from(if error.usage { 2 } else { 1 })
        }
    }
}

fn run(
    command: std::ffi::OsString,
    arguments: impl Iterator<Item = std::ffi::OsString>,
) -> Result<(), CliError> {
    if command == "list-compute-devices" {
        return list_compute_devices(arguments);
    }
    let Some(args) = Args::parse(command, arguments).map_err(CliError::usage)? else {
        print!("{}", args::HELP);
        return Ok(());
    };
    let voice = args.voice.resolve()?;
    if let Some(blend) = args
        .render
        .as_ref()
        .and_then(|render| render.blend.as_ref())
        && (!rynthv::synthesis::voice::flat::is_voice_path(&voice)
            || !rynthv::synthesis::voice::flat::is_voice_path(&blend.voice))
    {
        return Err("Flat blending requires two .sfpk voices".to_owned().into());
    }
    let start = Instant::now();
    let registry = if rynthv::synthesis::voice::flat::is_voice_path(&voice) {
        let mut registry = flat::registry(
            args.flat_directory.as_deref(),
            args.model_directory.as_deref(),
            Some(&voice),
        )?;
        if let Some(blend) = args
            .render
            .as_ref()
            .and_then(|render| render.blend.as_ref())
        {
            flat::include_neighbors(&mut registry, &blend.voice);
        }
        Some(registry)
    } else {
        None
    };
    let inventory = if let Some(registry) = &registry {
        VoiceInventory::load_flat(&voice, registry)
    } else {
        VoiceInventory::load(&voice)
    }
    .map_err(|e| e.to_string())?;
    let load_ms = start.elapsed().as_secs_f64() * 1000.0;
    if let Some(import) = args.context {
        return write_report(args.output, &import.run(&inventory)?).map_err(Into::into);
    }
    let mut preparation_ms = None;
    let preparation = if matches!(
        args.command,
        Command::Prepare | Command::RunModule | Command::ResolvePhone | Command::RenderScore
    ) {
        let start = Instant::now();
        let model = inventory.prepare().map_err(|e| e.to_string())?;
        preparation_ms = Some(start.elapsed().as_secs_f64() * 1000.0);
        Some(model)
    } else {
        None
    };
    let plan = if let Some(path) = args.project {
        let project = storage::load(&path)?.project;
        let mut planner = Planner::default();
        planner.update(&project, 0, |_| Some(inventory.identity().to_owned()));
        Some(planner.summary())
    } else {
        None
    };
    let execution = if let Some(module) = args.module {
        let bytes = read_json_bounded(&module.input, 16 * 1024 * 1024, "Module input JSON")?;
        let input: Vec<f32> = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Input must be a flat JSON array of finite f32 values: {e}"))?;
        if input.is_empty() || input.iter().any(|value| !value.is_finite()) {
            return Err("Module input must be nonempty and finite".into());
        }
        let compute = compute::Session::new(module.backend);
        compute.check()?;
        let start = Instant::now();
        let output = compute
            .scope(|| {
                preparation
                    .as_ref()
                    .expect("RunModule prepares a voice")
                    .execute_module(module.file, module.tree, module.node, &input)
            })
            .map_err(|e| e.to_string())?;
        if output.iter().any(|value| !value.is_finite()) {
            return Err(
                "Module returned nonfinite values; refusing JSON's lossy null conversion".into(),
            );
        }
        Some(json!({
            "file_index": module.file, "tree_index": module.tree, "node_index": module.node,
            "input_elements": input.len(), "output_elements": output.len(),
            "execution_ms": start.elapsed().as_secs_f64() * 1000.0,
            "values": output,
            "compute": compute.diagnostics(),
            "scope": "individual model module; intermediate features",
            "original_engine_bit_exact_verified": false,
        }))
    } else {
        None
    };
    let phone_lookup = if let Some(phone) = args.phone {
        let model = preparation.as_ref().expect("ResolvePhone prepares a voice");
        let result = model
            .resolve_phone(phone.file, phone.tree, &phone.language, &phone.token)
            .map_err(|e| e.to_string())?;
        Some(json!({ "language": phone.language, "token": phone.token, "resolved": result }))
    } else {
        None
    };
    let audio = if let Some(render_args) = &args.render {
        Some(render::run(
            render_args,
            &inventory,
            preparation.as_ref().expect("RenderScore prepares a voice"),
            registry.as_ref(),
        )?)
    } else {
        None
    };
    let report = json!({
        "schema": 2,
        "voice_load_ms": load_ms,
        "preparation_ms": preparation_ms,
        "voice": inventory,
        "preparation": preparation.as_ref().map(|model| model.summary()),
        "plan": plan,
        "execution": execution,
        "phone_lookup": phone_lookup,
        "audio_generated": audio.is_some(),
        "audio": audio,
    });
    write_report(args.output, &report).map_err(Into::into)
}

pub(crate) fn read_json_bounded(
    path: &std::path::Path,
    limit: u64,
    label: &str,
) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err(format!("{label} {} is not a regular file", path.display()));
    }
    if metadata.len() > limit {
        return Err(format!("{label} exceeds {} MiB", limit / (1024 * 1024)));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!("{label} exceeds {} MiB", limit / (1024 * 1024)));
    }
    Ok(bytes)
}

fn list_compute_devices(
    mut arguments: impl Iterator<Item = std::ffi::OsString>,
) -> Result<(), CliError> {
    let mut output = None;
    let mut validate = false;
    while let Some(option) = arguments.next() {
        match option.to_str() {
            Some("--validate") if !validate => validate = true,
            Some("--output") if output.is_none() => {
                output = Some(
                    arguments
                        .next()
                        .filter(|path| !path.to_string_lossy().starts_with("--"))
                        .ok_or_else(|| CliError::usage("Missing path for --output"))?
                        .into(),
                );
            }
            _ => {
                return Err(CliError::usage(
                    "Usage: list-compute-devices [--validate] [--output REPORT.json]",
                ));
            }
        }
    }
    let mut report = json!({ "cpu": true, "adapters": compute::list_devices() });
    if validate {
        let session = compute::Session::new(compute::Backend::WebGpu);
        report["shader_validation"] = json!({
            "compiled": session.validate_shaders().map_err(CliError::from)?,
            "device": session.status().device,
        });
    }
    write_report(output, &report).map_err(Into::into)
}

fn write_report(
    output: Option<std::path::PathBuf>,
    report: &serde_json::Value,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(report).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    if let Some(path) = output {
        storage::atomic_write(&path, &bytes)?;
    } else {
        io::stdout()
            .lock()
            .write_all(&bytes)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

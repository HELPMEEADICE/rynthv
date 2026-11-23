use super::args::RenderArgs;
use rynthv::synthesis::{
    audio::save_wav,
    engine,
    inference::compute,
    score::ExplicitScore,
    voice::{PreparedVoice, VoiceInventory, flat::FlatModelRegistry, session_identity},
};
use serde_json::{Value, json};
use std::time::Instant;

pub fn run(
    args: &RenderArgs,
    inventory: &VoiceInventory,
    voice: &PreparedVoice,
    registry: Option<&FlatModelRegistry>,
) -> Result<Value, String> {
    let bytes = super::read_json_bounded(&args.input, 16 * 1024 * 1024, "Explicit score JSON")?;
    let score: ExplicitScore = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Invalid explicit score: {error}"))?;
    let blend = args
        .blend
        .as_ref()
        .map(|blend| {
            let registry = registry.ok_or("Flat blend registry is unavailable")?;
            let other = VoiceInventory::load_flat(&blend.voice, registry)
                .map_err(|error| error.to_string())?;
            let model = other.prepare().map_err(|error| error.to_string())?;
            let blended = voice
                .blend_flat(&model, blend.weight)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((
                blended,
                json!({
                    "voice": blend.voice,
                    "weight": blend.weight,
                    "sha256": other.content_hash,
                }),
            ))
        })
        .transpose()?;
    let voice = blend.as_ref().map_or(voice, |(blended, _)| blended);
    let phrase = score.prepare(voice).map_err(|error| error.to_string())?;
    let name = &inventory
        .configuration
        .as_ref()
        .ok_or("Voice metadata is unavailable")?
        .name;
    let identity = if voice.family.requires_identity() {
        Some(match args.duration_identity {
            Some(identity) => identity,
            None => session_identity::saved_identity(name, voice.family.model_version())
                .map_err(|error| error.to_string())?,
        })
    } else if args.duration_identity.is_some() {
        return Err("原生 SV1 时长模型不接受 --duration-identity 条件。".into());
    } else {
        None
    };
    let compute = compute::Session::new(args.backend);
    compute.check()?;
    #[cfg(feature = "synth-profile")]
    if args.profile.is_some() {
        compute.enable_timestamps();
        rynthv::synthesis::profiling::start();
    }
    let mut output = None;
    let mut measurements = Vec::new();
    for iteration in 0..args.repeat.get() {
        let started = Instant::now();
        let mut first_pcm_ms = None;
        let mut chunks = 0;
        let mut last_stage = None;
        let render = compute
            .scope(|| {
                let checkpoint = |stage: engine::RenderStage| {
                    if iteration == 0 && last_stage != Some(stage.label()) {
                        eprintln!("合成：{}", stage.label());
                        last_stage = Some(stage.label());
                    }
                    Ok(())
                };
                if let Some(frames) = args.stream_frames {
                    engine::progressive::synthesize(
                        voice,
                        identity,
                        &phrase,
                        frames,
                        checkpoint,
                        |_| {
                            chunks += 1;
                            first_pcm_ms
                                .get_or_insert_with(|| started.elapsed().as_secs_f64() * 1000.0);
                        },
                    )
                } else {
                    engine::synthesize(voice, identity, &phrase, checkpoint)
                }
            })
            .map_err(|error| error.to_string());
        let diagnostics = compute.take_diagnostics();
        let failed = render.is_err();
        measurements.push(json!({
            "render_ms": started.elapsed().as_secs_f64() * 1000.0,
            "stage_timings": render.as_ref().ok().map(|render| &render.timings),
            "compute": diagnostics,
            "first_pcm_ms": first_pcm_ms,
            "pcm_chunks": chunks,
            "error": render.as_ref().err(),
        }));
        output = Some(render);
        if failed {
            break;
        }
    }
    #[cfg(feature = "synth-profile")]
    if let Some(path) = &args.profile {
        let mut profile = serde_json::to_value(rynthv::synthesis::profiling::finish())
            .map_err(|error| error.to_string())?;
        profile["renders"] = json!(measurements);
        let bytes = serde_json::to_vec_pretty(&profile).map_err(|error| error.to_string())?;
        rynthv::storage::atomic_write(path, &bytes)?;
    }
    let output = output.expect("repeat is nonzero")?;
    let last = measurements.last().expect("repeat is nonzero");
    let started = Instant::now();
    save_wav(&args.wav, &output.audio).map_err(|error| error.to_string())?;
    let wav_write_ms = started.elapsed().as_secs_f64() * 1000.0;
    let peak = output
        .audio
        .samples()
        .iter()
        .fold(0.0_f32, |peak, value| peak.max(value.abs()));
    let rms = if output.audio.samples().is_empty() {
        0.0
    } else {
        (output
            .audio
            .samples()
            .iter()
            .map(|&value| f64::from(value).powi(2))
            .sum::<f64>()
            / output.audio.samples().len() as f64)
            .sqrt()
    };
    Ok(json!({
        "wav": args.wav,
        "sample_rate": output.audio.sample_rate(),
        "channels": output.audio.layout().channels(),
        "frames": output.audio.frames(),
        "start_seconds": output.audio.start_seconds(),
        "duration_seconds": output.audio.duration_seconds(),
        "peak": peak,
        "rms": rms,
        "pitch": {
            "start_seconds": output.visuals.pitch.start_seconds,
            "frame_seconds": output.visuals.pitch.frame_seconds,
            "midi": output.visuals.pitch.values.as_ref(),
        },
        "phone_frame_counts": output.state.duration.durations.iter().map(|phone| phone.frames).collect::<Vec<_>>(),
        "flat_blend": blend.as_ref().map(|(_, report)| report),
        "render_ms": last["render_ms"],
        "compute": last["compute"],
        "renders": measurements,
        "wav_write_ms": wav_write_ms,
        "stage_timings": output.timings,
        "identity_source": if args.duration_identity.is_some() { "explicit model conditioning" } else if voice.family.requires_identity() { "saved RynthV context" } else { "not required" },
        "original_engine_bit_exact_verified": false,
    }))
}

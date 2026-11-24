use super::{analysis, *};
use crate::{
    model::{GroupRef, Project, TakeSet, Track},
    synthesis::{
        audio::{ChannelLayout, WaveFormat, save_wav_policy},
        execution::{self, Cancellation},
        offline::{Options, Progress},
    },
};
use std::{collections::BTreeSet, io::Write, ops::Range, sync::atomic::Ordering, time::Instant};

impl Workstation {
    pub(in crate::cli) fn render_command(&mut self, call: Call<'_>) -> Result<Value> {
        let path = call.path("project")?;
        let (mut project, _, _) = self.snapshot(path)?;
        let selection = Selection::from_open(self.current(path));
        let track = selection
            .track(call)
            .map(|id| target::track(&project, Some(id)))
            .transpose()?;
        let group = selection
            .group(call)
            .map(|id| {
                if let Some(track) = track {
                    return target::reference(track, Some(id));
                }
                project
                    .tracks
                    .iter()
                    .flat_map(Track::references)
                    .find(|reference| reference.uuid.eq_ignore_ascii_case(id))
                    .ok_or_else(|| Failure::new("GROUP_NOT_FOUND", "Group reference was not found"))
            })
            .transpose()?;
        let range = range(call, &project)?;
        let settings = resources::settings()?;
        let selected_tracks: BTreeSet<_> = track.into_iter().map(|track| track.id).collect();
        let options = Options {
            tracks: selected_tracks,
            group: group.map(|reference| reference.id),
            range_seconds: range.clone(),
            vocals_only: call.flag("vocals-only"),
            cache_bytes: call
                .number::<usize>("cache-mib")?
                .unwrap_or(512)
                .checked_mul(1024 * 1024)
                .ok_or_else(|| invalid("Cache size is too large"))?,
            backend: call
                .get("compute-backend")
                .map(resources::backend)
                .transpose()?
                .unwrap_or(settings.compute_backend),
            adapter: if let Some(adapter) = call.get("compute-adapter") {
                resources::adapter(adapter)?
            } else {
                settings.compute_adapter.clone()
            },
        };
        let analyze = call.action == "analyze";
        let outputs = outputs(call, &project, &options, analyze)?;
        let format = if analyze {
            WaveFormat {
                sample_rate: 48_000,
                layout: ChannelLayout::Stereo,
                bit_depth: 32,
            }
        } else {
            format(call, &project)?
        };
        let overwrite = call.flag("overwrite");
        for output in &outputs {
            separate_from_source(path, &output.path)?;
            check_output(&output.path, overwrite)?;
        }
        let seed = call.number::<u32>("seed")?;
        if call.flag("dry-run") {
            return Ok(
                json!({"dry_run":true,"outputs":outputs.iter().map(|output| &output.path).collect::<Vec<_>>(),
                "sample_rate":format.sample_rate,"channels":format.layout.channels(),"bit_depth":format.bit_depth,
                "compute_backend_requested":options.backend,"range_seconds":range}),
            );
        }
        if let Some(seed) = seed {
            override_seed(&mut project, seed)?;
        }
        let mut progress = BTreeSet::new();
        let mut load_times = Vec::new();
        let quiet = call.flag("quiet");
        let cancellation = Cancellation::from_flag(Arc::clone(&self.cancellation));
        let started = Instant::now();
        let result = self.offline.render(
            &project,
            &settings,
            &options,
            Arc::clone(&self.cancellation),
            |event| match event {
                Progress::VoiceLoaded { voice, elapsed_ms } => {
                    if !quiet {
                        let _ = writeln!(
                            io::stderr(),
                            "Loaded {} revision {} ({elapsed_ms:.0} ms)",
                            voice.database_id,
                            voice.revision
                        );
                    }
                    load_times.push(json!({"voice":voice,"inventory_load_ms":elapsed_ms}));
                }
                Progress::Stage { group, stage } => {
                    if !quiet && progress.insert((group.clone(), stage as u8)) {
                        let _ = writeln!(io::stderr(), "{group} {stage:?}");
                    }
                }
            },
        );
        cancelled(&self.cancellation)?;
        let output = result?;
        let mut exports = Vec::new();
        let mut measurements = None;
        if analyze {
            let mixed = execution::scope(cancellation.clone(), || output.mix(None));
            cancelled(&self.cancellation)?;
            let audio = mixed?.ok_or_else(no_audio)?;
            measurements = Some(analysis::measure(&audio));
        } else {
            for target in outputs {
                cancelled(&self.cancellation)?;
                let mixed = execution::scope(cancellation.clone(), || output.mix(target.track));
                cancelled(&self.cancellation)?;
                let Some(audio) = mixed.map_err(|error| {
                    Failure::new("MIX_FAILED", error).details(json!({"completed":exports}))
                })?
                else {
                    continue;
                };
                if let Some(parent) = target
                    .path
                    .parent()
                    .filter(|path| !path.as_os_str().is_empty())
                {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| Failure::new("OUTPUT_WRITE", error.to_string()))?;
                }
                let written = execution::scope(cancellation.clone(), || {
                    save_wav_policy(&target.path, &audio, format, overwrite)
                });
                if let Err(error) = written {
                    cancelled(&self.cancellation)?;
                    let message = error.to_string();
                    let code = if message.contains("OUTPUT_EXISTS: ") {
                        "OUTPUT_EXISTS"
                    } else {
                        "OUTPUT_WRITE"
                    };
                    return Err(Failure::new(code, message)
                        .details(json!({"path":target.path,"completed":exports})));
                }
                let frames = (audio.frames() as u128 * u128::from(format.sample_rate))
                    .div_ceil(u128::from(audio.sample_rate()));
                exports.push(json!({"path":target.path,"track_id":target.track_uuid,"duration_seconds":frames as f64 / f64::from(format.sample_rate)}));
            }
            if exports.is_empty() {
                return Err(no_audio());
            }
        }
        let report = output.report;
        let owners: Vec<_> = report.owners.iter().map(|item| {
            let mut data = json!({"group_ref_id":item.group_ref_id,"timings_ms":item.timings,
                "reused":item.timings.is_none(),"coverage":{"from_blick":item.owner.coverage.start.0,"to_blick":item.owner.coverage.end.0}});
            if analyze {
                let pitch = &item.owner.visuals.pitch;
                data["predicted_pitch"] = json!({"unit":"midi_semitones_without_voice_tuning","start_seconds":pitch.start_seconds,
                    "frame_seconds":pitch.frame_seconds,"values":pitch.values,"sounding_segments":pitch.segments});
                data["predicted_phonemes"] = json!(item.owner.visuals.phonemes.phones.iter().map(|phone| json!({
                    "note_id":item.note_ids.get(&phone.source.note_id),"phone_index":phone.source.phone_index,"token":phone.token,
                    "language":phone.language,"from_seconds":phone.begin_seconds,"to_seconds":phone.end_seconds,
                    "duration_seconds":phone.end_seconds-phone.begin_seconds
                })).collect::<Vec<_>>());
            }
            data
        }).collect();
        Ok(json!({
            "output":exports.first().and_then(|value| value.get("path")).filter(|_| exports.len() == 1),"outputs":exports,
            "sample_rate":if analyze {48_000} else {format.sample_rate},"channels":if analyze {2} else {format.layout.channels()},
            "bit_depth":if analyze {32} else {format.bit_depth},"duration_seconds":measurements.as_ref().and_then(|value| value.get("duration_seconds"))
                .or_else(|| exports.first().and_then(|value| value.get("duration_seconds")).filter(|_| exports.len() == 1)),
            "compute_backend_requested":options.backend,"compute_backend_actual":report.diagnostics.actual,
            "compute":report.diagnostics,"model_loads":load_times,"owners":owners,
            "rendered_owners":report.rendered_owners,"reused_owners":report.reused_owners,"acoustic_cache_bytes":report.cache_bytes,
            "render_ms":report.elapsed_ms,"total_ms":started.elapsed().as_secs_f64()*1000.0,
            "range_seconds":range,"measurements":measurements
        }))
    }
}

fn cancelled(flag: &AtomicBool) -> Result<()> {
    if flag.load(Ordering::Acquire) {
        return Err(Failure::new("CANCELLED", "Render cancelled"));
    }
    Ok(())
}
fn no_audio() -> Failure {
    Failure::new(
        "NO_AUDIO",
        "Selected sources have no audible audio. Check range, mute and solo settings.",
    )
}

fn range(call: Call<'_>, project: &Project) -> Result<Option<Range<f64>>> {
    if call.get("from").is_none() && call.get("to").is_none() {
        return Ok(None);
    }
    let unit = call.get("unit").unwrap_or("beat");
    let seconds = |key| {
        call.get(key)
            .map(|value| {
                let position = target::position(&project.time, unit, value)?;
                Ok::<_, Failure>(project.time.seconds_at(position))
            })
            .transpose()
    };
    let start = seconds("from")?.unwrap_or(0.0).max(0.0);
    let end = seconds("to")?.unwrap_or_else(|| project.time.seconds_at(project.end()));
    if !end.is_finite() || end <= start || end >= i64::MAX as f64 / 48_000.0 {
        return Err(invalid(
            "Render range must have a finite end after its start",
        ));
    }
    Ok(Some(start..end))
}
fn format(call: Call<'_>, project: &Project) -> Result<WaveFormat> {
    let n = |name: &str, key: &str| -> Result<u32> {
        if let Some(value) = call.number::<u32>(name)? {
            return Ok(value);
        }
        project
            .render_config
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                invalid(format!(
                    "Project export setting {key} is missing or invalid; pass --{name}"
                ))
            })
    };
    let rate = n("sample-rate", "sampleRate")?;
    let bit_depth = n("bit-depth", "bitDepth")?;
    let layout = match n("channels", "numChannels")? {
        1 => ChannelLayout::Mono,
        2 => ChannelLayout::Stereo,
        _ => return Err(invalid("Only mono and stereo export are supported")),
    };
    if ![44100, 48000, 96000].contains(&rate) || ![16, 24, 32].contains(&bit_depth) {
        return Err(invalid(
            "Use 44100, 48000 or 96000 Hz and 16, 24 or 32 bits",
        ));
    }
    Ok(WaveFormat {
        sample_rate: rate,
        layout,
        bit_depth: bit_depth as u16,
    })
}
struct Target {
    path: PathBuf,
    track: Option<u64>,
    track_uuid: Option<String>,
}
fn outputs(
    call: Call<'_>,
    project: &Project,
    options: &Options,
    analyze: bool,
) -> Result<Vec<Target>> {
    if analyze {
        if call.get("output").is_some()
            || call.get("output-dir").is_some()
            || call.flag("stems")
            || ["sample-rate", "channels", "bit-depth"]
                .iter()
                .any(|key| call.get(key).is_some())
        {
            return Err(invalid(
                "render analyze measures in-memory native 48000 Hz stereo audio. File format and stem options do not apply.",
            ));
        }
        return Ok(Vec::new());
    }
    if !call.flag("stems") {
        if call.get("output-dir").is_some() {
            return Err(invalid("--output-dir requires --stems"));
        }
        let path = call.path("output")?;
        return Ok(vec![Target {
            path: path.to_owned(),
            track: None,
            track_uuid: None,
        }]);
    }
    if call.get("output").is_some() {
        return Err(invalid("Use --output-dir for stems"));
    }
    let directory = call.path("output-dir")?;
    if directory.exists() && !directory.is_dir() {
        return Err(invalid("Stem output path must be a directory"));
    }
    let active_solo = project
        .tracks
        .iter()
        .any(|track| track.mixer.get("solo").and_then(Value::as_bool) == Some(true));
    let matching = |track: &Track, reference: &GroupRef| {
        options.group.is_none_or(|id| id == reference.id)
            && !reference.mute
            && project.group(&reference.group_id).is_some_and(|group| {
                group
                    .notes
                    .iter()
                    .any(|note| !note.muted() && reference.note_range(note).is_some())
            })
            && (!active_solo || track.mixer.get("solo").and_then(Value::as_bool) == Some(true))
    };
    Ok(project
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, track)| {
            if !options.tracks.is_empty() && !options.tracks.contains(&track.id) {
                return false;
            }
            if track.mixer.get("mute").and_then(Value::as_bool) == Some(true)
                || (active_solo && track.mixer.get("solo").and_then(Value::as_bool) != Some(true))
            {
                return false;
            }
            if track.is_instrumental() {
                !options.vocals_only && options.group.is_none() && track.audio_clip().is_some()
            } else {
                track
                    .references()
                    .any(|reference| matching(track, reference))
            }
        })
        .map(|(index, track)| {
            let safe: String = track
                .name
                .chars()
                .take(60)
                .map(|char| {
                    if char.is_control() || "<>:\"/\\|?*".contains(char) {
                        '_'
                    } else {
                        char
                    }
                })
                .collect();
            Target {
                path: directory.join(format!(
                    "{:03}-{}.wav",
                    index + 1,
                    safe.trim_end_matches(['.', ' '])
                )),
                track: Some(track.id),
                track_uuid: Some(track.main_ref.uuid.clone()),
            }
        })
        .collect())
}
fn override_seed(project: &mut Project, seed: u32) -> Result<()> {
    for group in project
        .library
        .iter_mut()
        .chain(project.tracks.iter_mut().map(|track| &mut track.main_group))
    {
        for note in &mut group.notes {
            let mut set = TakeSet::from_extra(&note.extra)?;
            let mut take = set.active().clone();
            take.id = set
                .takes
                .iter()
                .map(|take| take.id)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .filter(|id| *id <= i32::MAX as u32)
                .ok_or_else(|| invalid("Retake ID space exhausted"))?;
            take.seed_pitch = seed;
            take.seed_timbre = seed;
            set.active_take_id = take.id;
            set.takes.push(take);
            set.write_extra(&mut note.extra);
        }
    }
    Ok(())
}

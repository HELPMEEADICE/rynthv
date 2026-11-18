//! Opt-in tests using locally supplied voicebanks and pronunciation resources.
//! No model bytes or reference audio are distributed with the tests.

use std::path::PathBuf;

use rynthv::{
    model::{Note, Project, QUARTER},
    synthesis::{
        audio::{RenderedAudio, save_wav},
        engine,
        plan::Planner,
        score::{ExplicitScore, LyricFrontend, prepare_owner},
        voice::{VoiceFamily, VoiceInventory},
    },
};
use serde_json::json;

#[test]
#[ignore = "requires RVM_FLAT_SAMPLE and RVM_DICTIONARIES"]
fn real_flat_voice_renders_japanese_lyrics_and_vocal_modes() {
    let inventory = VoiceInventory::load(&sample("RVM_FLAT_SAMPLE")).unwrap();
    let voice = inventory.prepare().unwrap();
    let config = inventory.configuration.as_ref().unwrap();
    assert!(voice.summary().stage_programs_available);
    let lexicon = LyricFrontend::new(
        sample("RVM_DICTIONARIES"),
        &config.default_language,
        &config.supported_languages,
    )
    .unwrap();
    let mut project = Project::default();
    for (onset, pitch, lyric) in [(0, 60, "み"), (QUARTER, 64, "く"), (2 * QUARTER, 67, "あ")] {
        let mut note = Note::new(onset, QUARTER, pitch);
        note.lyrics = lyric.to_owned();
        project.tracks[0].main_group.notes.push(note);
    }
    let render = |project: &Project| {
        let mut planner = Planner::default();
        let plan = planner.update(project, 0, |_| Some(inventory.identity().to_owned()));
        let request = plan.requests.values().next().unwrap();
        let phrase = prepare_owner(&request.input, &lexicon, &voice).unwrap();
        assert!(
            phrase
                .records
                .iter()
                .flat_map(|record| &record.phones)
                .any(|phone| phone.language == "japanese" && phone.token == "m")
        );
        engine::synthesize(&voice, None, &phrase, |_| Ok(())).unwrap()
    };
    let neutral = render(&project);
    let name = config.styles.first().expect("sample declares a Vocal Mode");
    project.tracks[0].main_ref.extra.insert(
        "voice".into(),
        json!({
            "vocalModeParams": { (name): { "timbre": 100.0 } }
        }),
    );
    let styled = render(&project);
    assert_eq!(neutral.state.pitch.pitch, styled.state.pitch.pitch);
    assert_eq!(neutral.audio.frames(), styled.audio.frames());
    assert_ne!(neutral.audio.samples(), styled.audio.samples());
    verify_audio(&styled.audio);
    if let Some(path) = std::env::var_os("RVM_FLAT_WAV") {
        save_wav(&PathBuf::from(path), &styled.audio).unwrap();
    }
}

#[test]
#[ignore = "requires RVM_SV1_SAMPLE"]
fn real_sv1_nofs_still_synthesizes() {
    let inventory = VoiceInventory::load(&sample("RVM_SV1_SAMPLE")).unwrap();
    assert!(inventory.flat.is_none());
    let voice = inventory.prepare().unwrap();
    assert_eq!(voice.family, VoiceFamily::Sv1);
    let score: ExplicitScore =
        serde_json::from_str(include_str!("../examples/synthesis/mandarin-phrase.json")).unwrap();
    let phrase = score.prepare(&voice).unwrap();
    let output = engine::synthesize(&voice, None, &phrase, |_| Ok(())).unwrap();
    verify_audio(&output.audio);
}

#[test]
#[ignore = "requires RVM_SV2_REVISION with a supported DNNI v3 model"]
fn real_sv2_dnni_revision_still_prepares() {
    let inventory = VoiceInventory::load(&sample("RVM_SV2_REVISION")).unwrap();
    assert!(inventory.flat.is_none());
    let voice = inventory.prepare().unwrap();
    assert!(voice.family.requires_identity());
    let summary = voice.summary();
    assert!(
        summary.stage_programs_available,
        "{:?}",
        summary.stage_preparation_error
    );
}

fn sample(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("Set {name} to a local resource"))
}

fn verify_audio(audio: &RenderedAudio) {
    assert_eq!(audio.sample_rate(), 48_000);
    assert_eq!(audio.layout().channels(), 1);
    assert!(audio.frames() > 48_000);
    let peak = audio
        .samples()
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0, f32::max);
    assert!(audio.samples().iter().all(|sample| sample.is_finite()));
    assert!(peak > 0.01);
    println!(
        "{} frames, {:.3} seconds, peak {peak:.6}",
        audio.frames(),
        audio.duration_seconds()
    );
}

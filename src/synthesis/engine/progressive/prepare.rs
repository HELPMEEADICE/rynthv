use super::{PcmStore, PreparedRender};
use crate::synthesis::{
    engine::{
        OwnerSynthesis, PreparedModelFeatures, PreparedRenderAcoustics, RenderStage,
        incremental::{IncrementalEdit, acoustics},
        models, prepare_acoustics,
    },
    plan::OwnerInput,
    rendered::RenderedPhonemeTrack,
    score::{self, HOP_SAMPLES, LyricFrontend, PreparedPhrase, SAMPLE_RATE},
    voice::{PreparedVoice, Result, VoiceError, VoiceFamily},
};

/// Both voice generations prepare duration and pitch before choosing a demand
/// position. Frame-level acoustics and PCM remain deferred to streaming cursors.
pub fn prepare(
    voice: &PreparedVoice,
    identity: Option<u64>,
    input: &OwnerInput,
    lexicon: &LyricFrontend,
    base: Option<(&OwnerInput, &OwnerSynthesis)>,
    mut checkpoint: impl FnMut(RenderStage) -> Result<()>,
) -> Result<PreparedRender> {
    let phrase = score::prepare_owner(input, lexicon, voice)?;
    // Local invalidation currently uses the SV2 feature program. Compatible
    // voices still retain progressive PCM, but prepare a fresh owner on edits.
    let base = base.filter(|(_, state)| {
        voice.family == VoiceFamily::Sv2
            && matches!(state.prepared.features, PreparedModelFeatures::Sv2 { .. })
    });
    let edit = base
        .map(|(previous, state)| {
            IncrementalEdit::between(previous, input, &state.prepared.phrase, &phrase)
        })
        .transpose()?
        .flatten()
        .filter(|_| {
            let (_, previous) = base.expect("an edit requires a baseline");
            let pcm = &previous.components;
            previous.pitch.pitch.len().checked_mul(HOP_SAMPLES) == Some(pcm.primary().len())
                && ((pcm.start_seconds - phrase.start_seconds) * f64::from(SAMPLE_RATE)).abs()
                    < 0.0001
        });
    if let Some(((_, previous), edit)) = base.zip(edit) {
        let updated = acoustics::recompute(
            voice,
            models::required_identity(identity)?,
            &phrase,
            previous,
            &edit,
            &mut checkpoint,
        )?;
        let pcm = PcmStore::from_existing(&previous.components, edit.first_block..edit.stop_block)?;
        prepared(
            voice,
            PreparedRenderAcoustics {
                prepared: updated.state.prepared,
                duration: updated.state.duration,
                pitch: updated.state.pitch,
                timings: updated.timings,
            },
            pcm,
        )
    } else {
        prepare_phrase(voice, identity, &phrase, checkpoint)
    }
}

pub(super) fn prepare_phrase(
    voice: &PreparedVoice,
    identity: Option<u64>,
    phrase: &PreparedPhrase,
    checkpoint: impl FnMut(RenderStage) -> Result<()>,
) -> Result<PreparedRender> {
    let acoustic = prepare_acoustics(voice, identity, phrase, checkpoint)?;
    let frames = match acoustic.prepared.features {
        PreparedModelFeatures::Sv1Local => acoustic
            .prepared
            .phones
            .iter()
            .try_fold(0_usize, |total, phone| total.checked_add(phone.frames)),
        PreparedModelFeatures::Sv2 { .. } => Some(acoustic.pitch.pitch.len()),
    }
    .ok_or_else(|| VoiceError::new("Progressive acoustic frame count overflow"))?;
    let samples = frames
        .checked_mul(HOP_SAMPLES)
        .ok_or_else(|| VoiceError::new("Progressive PCM size overflow"))?;
    let pcm = PcmStore::new(phrase.start_seconds, samples)?;
    prepared(voice, acoustic, pcm)
}

fn prepared(
    voice: &PreparedVoice,
    acoustic: PreparedRenderAcoustics,
    pcm: PcmStore,
) -> Result<PreparedRender> {
    let pitch = models::pitch_track(voice, &acoustic.prepared.phrase, acoustic.pitch.clone())?;
    let phonemes =
        RenderedPhonemeTrack::from_phrase(&acoustic.prepared.phrase, &acoustic.duration)?;
    Ok(PreparedRender {
        acoustic,
        pcm,
        pitch,
        phonemes,
        timbre_frames: Default::default(),
    })
}

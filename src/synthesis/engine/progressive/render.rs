use super::{Network, prepare};
use crate::synthesis::{
    audio::RenderedAudio,
    engine::{RenderResult, RenderStage},
    score::PreparedPhrase,
    voice::{PreparedVoice, Result, VoiceError},
};
use std::num::NonZeroUsize;

/// Run the same bounded PCM producer as the editor for an explicit phrase.
pub fn synthesize(
    voice: &PreparedVoice,
    identity: Option<u64>,
    phrase: &PreparedPhrase,
    max_frames: NonZeroUsize,
    mut checkpoint: impl FnMut(RenderStage) -> Result<()>,
    mut publish: impl FnMut(&RenderedAudio),
) -> Result<RenderResult> {
    let mut prepared = prepare::prepare_phrase(voice, identity, phrase, &mut checkpoint)?;
    let network = Network::prepare(voice)?;
    while !prepared.is_complete() {
        let mut cursor = prepared
            .start(&network, voice, phrase.start_seconds)?
            .ok_or_else(|| VoiceError::new("Progressive render has no cursor for pending PCM"))?;
        while !cursor.is_finished() {
            for audio in prepared.advance(&mut cursor, max_frames.get(), &mut checkpoint)? {
                publish(&audio);
            }
        }
    }
    prepared.finish()
}

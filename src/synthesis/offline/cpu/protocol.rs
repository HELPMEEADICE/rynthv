use super::super::{RenderStage, StageTimings, SynthesisSettings};
use crate::synthesis::{
    plan::OwnerInput,
    rendered::{PitchTrack, RenderedPhonemeTrack},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

pub(crate) type Activity = BTreeMap<String, BTreeMap<String, u64>>;

#[derive(Deserialize, Serialize)]
pub(crate) struct Job {
    pub(super) serial: u64,
    pub(super) baseline: Option<u64>,
    pub(super) voice: PathBuf,
    pub(super) settings: SynthesisSettings,
    pub(super) input: Arc<OwnerInput>,
    pub(super) start_seconds: f64,
    pub(super) preview: bool,
    pub(crate) demand: Option<f64>,
}

#[derive(Deserialize, Serialize)]
pub(in super::super) enum Update {
    Model {
        voice_load_ms: Option<f64>,
        model_prepare_ms: f64,
    },
    Stage(RenderStage),
    Prepared {
        first: i64,
        end: i64,
    },
    Pcm {
        seconds: f64,
    },
}

#[derive(Deserialize, Serialize)]
pub(super) struct Completed {
    pub(super) pcm_origin: f64,
    pub(super) visible_begin: f64,
    pub(super) visible_end: Option<f64>,
    pub(super) pitch: PitchTrack,
    pub(super) phonemes: RenderedPhonemeTrack,
    pub(super) timings: StageTimings,
    pub(super) activity: Activity,
}

//! Shared DDS acoustic model with distinct SV1 and SV2-compatible adapters.

mod decoder;
mod frontend;
mod noise;
mod pitch;
mod prepared;
mod queue;
mod records;
mod sampling;
mod stream;
mod styles;

use std::sync::Arc;

pub use prepared::PreparedSv1TimbreFrames;
pub use stream::Sv1TimbreStream;

use crate::synthesis::{
    stages::{
        f0::PitchResult,
        timbre::{TimbreOutput, TimbreRequest, TimbreStyles},
    },
    voice::{PreparedTree, Result, VoiceError, embedded::types, flat::FlatConditioning},
};
use frontend::{Context, Frontend};
use sampling::Sampler;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Contract {
    Native,
    Compatible,
}

pub struct Sv1TimbreNetwork<'a> {
    contract: Contract,
    tree: &'a PreparedTree,
    frontend: Frontend<'a>,
    context: Context<'a>,
    sampler: Sampler,
    bank: &'a [f32],
    styles: TimbreStyles,
    decoder: [usize; 4],
}

impl<'a> Sv1TimbreNetwork<'a> {
    pub fn frame_seconds(&self) -> f64 {
        f64::from(self.frontend.frame_seconds)
    }

    pub fn prepare(tree: &'a PreparedTree) -> Result<Self> {
        Self::prepare_contract(tree, Contract::Compatible, None)
    }

    pub fn prepare_native(tree: &'a PreparedTree) -> Result<Self> {
        Self::prepare_native_conditioned(tree, None)
    }

    pub(crate) fn prepare_native_conditioned(
        tree: &'a PreparedTree,
        flat: Option<&'a FlatConditioning>,
    ) -> Result<Self> {
        Self::prepare_contract(tree, Contract::Native, flat)
    }

    fn prepare_contract(
        tree: &'a PreparedTree,
        contract: Contract,
        flat: Option<&'a FlatConditioning>,
    ) -> Result<Self> {
        let root = tree.node(tree.root_index())?;
        if root.type_id != types::TIMBRE_TYPE_ID
            || !tree.payload(tree.root_index())?.is_empty()
            || root.children.len() != 2
        {
            return Err(VoiceError::new(
                "Not a supported version-1 compatible timbre root",
            ));
        }
        let program = tree.node(root.children[0])?;
        if program.type_id != types::TIMBRE_SAMPLER_TYPE_ID
            || !tree.payload(root.children[0])?.is_empty()
        {
            return Err(VoiceError::new("Unsupported compatible acoustic program"));
        }
        let nodes: [usize; 8] =
            program.children.as_slice().try_into().map_err(|_| {
                VoiceError::new("Compatible acoustic program requires eight children")
            })?;
        let stored_bank = tree
            .tensor(nodes[1])?
            .vector()
            .ok_or_else(|| VoiceError::new("Compatible acoustic bank is not a vector"))?;
        let stored_styles = TimbreStyles::prepare(tree, root.children[1])?;
        let (bank, styles) = if let Some(flat) = flat {
            if flat.base.len() != stored_bank.len()
                || flat.styles.neutral.len() != stored_styles.neutral.len()
            {
                return Err(VoiceError::new(
                    "Flat style embeddings differ from the DDS acoustic model",
                ));
            }
            (flat.base.as_slice(), flat.styles.clone())
        } else {
            (stored_bank, stored_styles)
        };
        Ok(Self {
            contract,
            tree,
            frontend: Frontend::prepare(tree, nodes[0], contract)?,
            context: Context::prepare(tree, nodes[2])?,
            sampler: Sampler::prepare(tree, nodes[3])?,
            bank,
            styles,
            decoder: nodes[4..].try_into().unwrap(),
        })
    }

    /// Phone projection uses the complete owner, as in SV2's duration pass.
    /// Framewise context, latent and decoder networks run only when requested.
    pub fn prepare_frame_features(
        &self,
        request: &TimbreRequest<'_>,
        pitch: &PitchResult,
    ) -> Result<PreparedSv1TimbreFrames> {
        PreparedSv1TimbreFrames::prepare(self, request, pitch)
    }

    pub fn start_prepared<'network>(
        &'network self,
        request: TimbreRequest<'_>,
        frames: Arc<PreparedSv1TimbreFrames>,
    ) -> Result<Sv1TimbreStream<'network, 'a>> {
        Sv1TimbreStream::new(self, &request, frames)
    }

    /// Exports and preview share the same persistent numerical program.
    pub fn render(&self, request: &TimbreRequest<'_>, pitch: &PitchResult) -> Result<TimbreOutput> {
        let frames = Arc::new(self.prepare_frame_features(request, pitch)?);
        Sv1TimbreStream::new(self, request, frames)?.collect()
    }
}

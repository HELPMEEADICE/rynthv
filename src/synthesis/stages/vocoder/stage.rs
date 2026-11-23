//! Voice-specific vocoder programs over shared prepared model weights.

use crate::synthesis::{
    inference::Sequence,
    voice::{PreparedModule, PreparedTree, Result, VoiceError},
};

use super::{
    condition::VocoderCondition,
    configuration::{Configuration, Modules},
    excitation::ExcitationFrame,
    gain_filter::GainFilter,
    interpolation::{SpectralFilterConfig, SpectralFilterStream},
    output::FrameMetadata,
    postfilter::Postfilter,
    sampling::PositionalGru,
    spectral_heads::SpectralHeads,
    stream_parameters::ParameterStream,
    voice_control::{Component, SpectralControl, VoiceControl},
};

#[derive(Clone)]
pub(crate) struct VocoderParameterStage<'a> {
    pub(crate) tree: &'a PreparedTree,
    pub(crate) config: Configuration,
    pub(crate) modules: Modules,
    pub(crate) condition: VocoderCondition,
    pub(crate) positional: Option<PositionalGru>,
    pub(crate) heads: SpectralHeads,
}

#[derive(Clone)]
pub(crate) struct VocoderSignalStage {
    pub(crate) gain: GainFilter,
    pub(crate) harmonic_filter: SpectralFilterStream,
    pub(crate) modulated_filter: SpectralFilterStream,
    pub(crate) noise_filter: SpectralFilterStream,
    pub(crate) postfilter: Postfilter,
}

#[derive(Clone)]
pub struct VocoderStage<'a> {
    pub(crate) params: VocoderParameterStage<'a>,
    pub(crate) signals: VocoderSignalStage,
}

/// Independently materialized neural outputs used by the signal processors.
/// These are not PCM and do not bypass the learned frequency responses.
pub struct VocoderParameters {
    pub normalized: Sequence,
    pub spectra: [Sequence; 3],
    pub harmonic_parameters: Sequence,
    pub excitation: Vec<ExcitationFrame>,
    pub metadata: Vec<FrameMetadata>,
    pub(crate) voice_controls: Vec<SpectralControl>,
}

impl<'a> VocoderStage<'a> {
    pub fn prepare(tree: &'a PreparedTree) -> Result<Self> {
        #[cfg(feature = "synth-profile")]
        let _profile = crate::synthesis::profiling::span("vocoder.prepare");
        let root = tree.root_index();
        let config = Configuration::parse(&tree.payload(root)?)?;
        let modules = Modules::prepare(tree, root, config)?;
        let condition = VocoderCondition::prepare(tree, modules.conditions)?;
        if config.bands != 8 || modules.input_projection.is_none() {
            return Err(VoiceError::new("Unsupported vocoder source variant"));
        }
        let main = match &tree.node(modules.conditioned_network)?.module {
            PreparedModule::Conditional(config) => *config,
            _ => {
                return Err(VoiceError::new(
                    "Vocoder main network must be a prepared conditional stack",
                ));
            }
        };
        let positional = modules
            .recurrent_sampler
            .map(|sampler| PositionalGru::prepare(tree, sampler))
            .transpose()?;
        // SV1 executes its [72, 256, 0] main network
        // directly. SV2's additional positional child supplies the real 192
        // condition channels. A missing child must never become zero padding.
        let condition_channels = match &positional {
            None => 0,
            Some(positional) if positional.output_channels() == 192 => 192,
            Some(_) => {
                return Err(VoiceError::new(
                    "Unsupported vocoder positional condition width",
                ));
            }
        };
        if main.input_channels != 72
            || main.output_channels != 256
            || main.condition_channels != condition_channels
            || condition.frame_samples != 240
        {
            return Err(VoiceError::new("Unexpected vocoder condition dimensions"));
        }
        let heads = SpectralHeads::prepare(tree, modules.spectral_heads)?;
        if heads.head_count() != 3 || heads.channels() != 512 {
            return Err(VoiceError::new(
                "Vocoder requires three 512-channel spectral heads",
            ));
        }
        let hop = condition.frame_samples;
        let period = condition.frame_seconds;
        let make_filter = |node, component| {
            SpectralFilterStream::new(
                SpectralFilterConfig::prepare(tree, node)?,
                hop,
                condition.sample_rate,
                component,
            )
        };
        let gain = GainFilter::prepare(tree, modules.coefficients, hop, period)?;
        let harmonic_filter = make_filter(modules.harmonic_filter, Component::Primary)?;
        let modulated_filter = make_filter(modules.third_filter, Component::Primary)?;
        let noise_filter = make_filter(modules.noise_filter, Component::Secondary)?;
        let postfilter = Postfilter::prepare(tree, &modules, hop, config.gate_postfilter_noise)?;
        Ok(Self {
            params: VocoderParameterStage {
                tree,
                config,
                modules,
                condition,
                positional,
                heads,
            },
            signals: VocoderSignalStage {
                gain,
                harmonic_filter,
                modulated_filter,
                noise_filter,
                postfilter,
            },
        })
    }

    pub fn parameters(
        &self,
        physical: &Sequence,
        controls: Option<&Sequence>,
        voice_controls: Option<&[VoiceControl]>,
    ) -> Result<VocoderParameters> {
        ParameterStream::new(self.params.clone())?.process(physical, controls, voice_controls, true)
    }
}

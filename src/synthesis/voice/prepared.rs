//! Typed module preparation, not a generic dataflow interpreter.
//!
//! Children in the source arena describe ownership. Unknown parent modules retain
//! those children without inventing the order in which tensors flow through them.

mod affine;
mod convolution;
mod duration;
mod factory;
mod phones;
mod source;
mod summary;
mod transposed_convolution;

pub use affine::PreparedAffine;
pub use convolution::PreparedConvolution;
pub use duration::DurationInputLayout;
pub(crate) use phones::PhoneResolver;
pub use summary::*;
pub use transposed_convolution::PreparedTransposedConvolution;

use std::sync::Arc;

use crate::synthesis::inference::{
    ModelRuntime, Sequence, UnaryOperator,
    affine::Affine,
    ops::{
        attention::MultiHeadAttention,
        attention_relative::{AttentionOptions, RelativeAttention},
        bidirectional::BidirectionalGru,
        conditional::ConditionalConfig,
        embedding::Embedding,
        gated::GatedConfig,
        normalization::{ChannelAffine, LayerNorm},
        parameterized::LeakyRelu,
        range_normalizer::RangeNormalizer,
        standard_normalizer::StandardNormalizer,
    },
    pooling::PoolConfig,
    recurrent::Gru,
};

use super::{
    DurationConfiguration, Result, VoiceError, hex,
    phones::PhoneSet,
    tensor::{TensorLayout, TensorStorage},
};

pub struct PreparedTree {
    nodes: Vec<PreparedNode>,
    root: usize,
    duration_input: Option<DurationInputLayout>,
}

pub struct PreparedNode {
    pub resource_index: usize,
    pub type_id: [u8; 16],
    pub children: Vec<usize>,
    pub payload_bytes: usize,
    payload: Option<Box<[u8]>>,
    pub module: PreparedModule,
}

pub enum PreparedModule {
    Tensor(Arc<TensorStorage>),
    Unary(UnaryOperator),
    LeakyRelu(LeakyRelu),
    Affine(PreparedAffine),
    PhoneSet(PhoneSet),
    Convolution(PreparedConvolution),
    TransposedConvolution(PreparedTransposedConvolution),
    Gru(Gru),
    BidirectionalGru(Box<BidirectionalGru>),
    Pool(PoolConfig),
    Sequential,
    ModuleList,
    Embedding(Embedding),
    LayerNorm(LayerNorm),
    ChannelAffine(ChannelAffine),
    RangeNormalizer(RangeNormalizer),
    StandardNormalizer(StandardNormalizer),
    RelativeAttention(RelativeAttention),
    Attention(MultiHeadAttention),
    Conditional(ConditionalConfig),
    Gated(GatedConfig),
    SamplerNetwork,
    Unsupported,
}

impl PreparedTree {
    pub fn nodes(&self) -> &[PreparedNode] {
        &self.nodes
    }

    pub fn root_index(&self) -> usize {
        self.root
    }

    pub fn node(&self, index: usize) -> Result<&PreparedNode> {
        self.nodes
            .get(index)
            .ok_or_else(|| VoiceError::new("Model node index is out of range"))
    }

    pub(crate) fn duration_configuration(&self) -> Result<DurationConfiguration> {
        self.duration_input
            .as_ref()
            .map(|layout| layout.configuration)
            .ok_or_else(|| VoiceError::new("Expected a duration root"))
    }

    pub fn payload(&self, index: usize) -> Result<&[u8]> {
        self.node(index)?
            .payload
            .as_deref()
            .ok_or_else(|| VoiceError::new("Raw tensor payload is unavailable after preparation"))
    }

    pub fn tensor(&self, index: usize) -> Result<&TensorStorage> {
        match &self.node(index)?.module {
            PreparedModule::Tensor(tensor) => Ok(tensor),
            _ => Err(VoiceError::new("Expected a tensor storage node")),
        }
    }

    pub fn affine(&self, index: usize) -> Result<&Affine> {
        match &self.node(index)?.module {
            PreparedModule::Affine(layer) => Ok(layer.operator()),
            _ => Err(VoiceError::new("Expected an affine node")),
        }
    }

    pub fn range_normalizer(&self, index: usize) -> Result<&RangeNormalizer> {
        match &self.node(index)?.module {
            PreparedModule::RangeNormalizer(layer) => Ok(layer),
            _ => Err(VoiceError::new("Expected a range normalizer node")),
        }
    }

    pub fn standard_normalizer(&self, index: usize) -> Result<&StandardNormalizer> {
        match &self.node(index)?.module {
            PreparedModule::StandardNormalizer(layer) => Ok(layer),
            _ => Err(VoiceError::new("Expected a standard normalizer node")),
        }
    }

    pub fn attention(&self, index: usize) -> Result<&MultiHeadAttention> {
        match &self.node(index)?.module {
            PreparedModule::Attention(layer) => Ok(layer),
            _ => Err(VoiceError::new("Expected an attention node")),
        }
    }

    pub fn execute_attention(
        &self,
        index: usize,
        queries: &[f32],
        keys: &[f32],
        values: &[f32],
        options: AttentionOptions<'_>,
    ) -> Result<Vec<f32>> {
        self.attention(index)?
            .execute(queries, keys, values, options)
            .map_err(|error| VoiceError::new(error.to_string()))
    }

    /// Run a complete independent sequence for diagnostics or an offline stage.
    /// Stateful chunked execution must use an explicitly owned ModelRuntime.
    pub fn execute_node(&self, index: usize, input: &[f32]) -> Result<Vec<f32>> {
        let mut runtime = ModelRuntime::new(self);
        let channels = match runtime.input_channels(index)? {
            Some(channels) => channels,
            None if runtime.flat_elementwise(index)? => 1,
            None => {
                return Err(VoiceError::new(format!(
                    "Model node {index} type {} needs an explicitly shaped Sequence",
                    hex(&self.node(index)?.type_id)
                )));
            }
        };
        let input = Sequence::from_values(channels, input.to_vec())?;
        Ok(runtime.execute(index, &input, true)?.values)
    }
}

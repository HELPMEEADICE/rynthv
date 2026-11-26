//! Stateful reset-after GRU with prepared gate biases and reusable frame buffers.

#[path = "recurrent/tanh.rs"]
mod tanh;

use std::{
    fmt,
    sync::{Arc, OnceLock},
};

use super::{
    InferenceError,
    activation::ActivationOperator,
    affine::Affine,
    gpu::Resident,
    matrix::{MatrixError, check_length},
};

pub const GRU_TYPE_ID: [u8; 16] = [
    0x06, 0xcb, 0x32, 0xfa, 0x71, 0xcd, 0x89, 0x69, 0x0b, 0x18, 0xa3, 0x8d, 0x38, 0xd7, 0xaa, 0x5d,
];

#[derive(Debug)]
pub enum RecurrentError {
    Matrix(MatrixError),
    Activation(InferenceError),
}

impl fmt::Display for RecurrentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Matrix(error) => error.fmt(f),
            Self::Activation(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for RecurrentError {}

impl From<MatrixError> for RecurrentError {
    fn from(error: MatrixError) -> Self {
        Self::Matrix(error)
    }
}

impl From<InferenceError> for RecurrentError {
    fn from(error: InferenceError) -> Self {
        Self::Activation(error)
    }
}

/// Immutable, shareable GRU weights. Runtime state belongs to each render stream.
#[derive(Clone)]
pub struct Gru {
    // Input reset, input update, input candidate; then recurrent reset,
    // recurrent update, recurrent candidate, exactly as stored in the tree.
    layers: [Affine; 6],
    gpu_weights: Arc<OnceLock<Arc<[f32]>>>,
}

/// A resumable frame-boundary state, including reusable execution buffers.
/// Cloning provides an independent checkpoint; immutable weights stay in Gru.
#[derive(Clone)]
pub struct GruState {
    hidden: Vec<f32>,
    resident: Option<Resident>,
    reset: Vec<f32>,
    update: Vec<f32>,
    candidate: Vec<f32>,
    scratch: Vec<f32>,
}

impl GruState {
    pub fn hidden(&self) -> &[f32] {
        &self.hidden
    }

    pub(super) fn resident(&self) -> Option<&Resident> {
        self.resident.as_ref()
    }

    pub fn reset(&mut self) {
        self.resident = None;
        self.hidden.fill(0.0);
        self.reset.fill(0.0);
        self.update.fill(0.0);
        self.candidate.fill(0.0);
        self.scratch.fill(0.0);
    }

    pub fn restore_hidden(&mut self, hidden: &[f32]) -> Result<(), MatrixError> {
        check_length("GRU checkpoint", self.hidden.len(), hidden.len())?;
        self.reset();
        self.hidden.copy_from_slice(hidden);
        Ok(())
    }
}

impl Gru {
    /// Six affine pairs in serialized order: children (0,1), (2,3), ... (10,11).
    /// Every pair must include its bias. Gate biases 0+3 and 1+4 are merged
    /// once before execution.
    pub fn new(serialized_layers: [Affine; 6]) -> Result<Self, MatrixError> {
        let inputs = serialized_layers[0].input_channels();
        let hidden = serialized_layers[0].output_channels();
        for (index, layer) in serialized_layers.iter().enumerate() {
            if layer.output_channels() != hidden
                || layer.input_channels() != if index < 3 { inputs } else { hidden }
                || layer.bias().is_none()
            {
                return Err(MatrixError::InvalidShape);
            }
        }
        let merge = |first: usize, second: usize| -> Arc<[f32]> {
            serialized_layers[first]
                .bias()
                .expect("bias shapes validated")
                .iter()
                .zip(
                    serialized_layers[second]
                        .bias()
                        .expect("bias shapes validated"),
                )
                .map(|(left, right)| left + right)
                .collect()
        };
        let reset_bias = merge(0, 3);
        let update_bias = merge(1, 4);
        let [
            input_reset,
            input_update,
            input_candidate,
            hidden_reset,
            hidden_update,
            hidden_candidate,
        ] = serialized_layers;
        Ok(Self {
            layers: [
                Affine::new(input_reset.matrix().clone(), Some(reset_bias))?,
                Affine::new(input_update.matrix().clone(), Some(update_bias))?,
                input_candidate,
                Affine::new(hidden_reset.matrix().clone(), None)?,
                Affine::new(hidden_update.matrix().clone(), None)?,
                hidden_candidate,
            ],
            gpu_weights: Arc::default(),
        })
    }

    pub fn input_channels(&self) -> usize {
        self.layers[0].input_channels()
    }

    pub fn output_channels(&self) -> usize {
        self.layers[0].output_channels()
    }

    pub fn new_state(&self) -> GruState {
        let hidden = self.output_channels();
        GruState {
            hidden: vec![0.0; hidden],
            resident: None,
            reset: vec![0.0; hidden],
            update: vec![0.0; hidden],
            candidate: vec![0.0; hidden],
            scratch: vec![0.0; hidden],
        }
    }

    /// Consume one frame without allocating. The new hidden state is the output.
    pub fn step(&self, input: &[f32], state: &mut GruState) -> Result<(), RecurrentError> {
        check_length("GRU input", self.input_channels(), input.len())?;
        self.check_state(state)?;
        state.resident = None;
        self.step_validated(input, state)
    }

    /// Consume a contiguous, frame-major chunk, retaining the final hidden state
    /// for the next chunk. All shapes are checked before any state/output change.
    pub fn execute_sequence(
        &self,
        input: &[f32],
        output: &mut [f32],
        frames: usize,
        state: &mut GruState,
    ) -> Result<(), RecurrentError> {
        #[cfg(feature = "synth-profile")]
        let _profile = crate::synthesis::profiling::span("op.gru.sequence");
        let inputs = frames
            .checked_mul(self.input_channels())
            .ok_or(MatrixError::InvalidShape)?;
        let outputs = frames
            .checked_mul(self.output_channels())
            .ok_or(MatrixError::InvalidShape)?;
        check_length("GRU sequence input", inputs, input.len())?;
        check_length("GRU sequence output", outputs, output.len())?;
        self.check_state(state)?;
        if frames == 0 {
            return Ok(());
        }
        if let Some(next) = super::gpu::gru(self, input, output, state, frames)? {
            state.restore_hidden(&output[output.len() - self.output_channels()..])?;
            state.resident = Some(next);
            return Ok(());
        }
        state.resident = None;
        let cancellation = crate::synthesis::execution::current();
        for (frame, (input, output)) in input
            .chunks_exact(self.input_channels())
            .zip(output.chunks_exact_mut(self.output_channels()))
            .enumerate()
        {
            if frame.is_multiple_of(32)
                && cancellation
                    .as_ref()
                    .is_some_and(|token| token.is_cancelled())
            {
                return Err(MatrixError::Cancelled.into());
            }
            self.step_validated(input, state)?;
            output.copy_from_slice(&state.hidden);
        }
        Ok(())
    }

    pub(super) fn gpu_weight_len(&self) -> usize {
        self.input_channels()
            .saturating_add(self.output_channels())
            .saturating_add(2)
            .saturating_mul(self.output_channels())
            .saturating_mul(4)
    }

    pub(super) fn gpu_weights(&self) -> &Arc<[f32]> {
        self.gpu_weights.get_or_init(|| {
            let mut packed = Vec::with_capacity(self.gpu_weight_len());
            // One aligned vector load supplies all three gates for a connection.
            for layers in self.layers.as_chunks::<3>().0 {
                let hidden = self.output_channels();
                for column in 0..layers[0].input_channels() {
                    for channel in 0..hidden {
                        let [reset, update, candidate] = layers
                            .each_ref()
                            .map(|layer| layer.matrix().row(channel)[column]);
                        packed.extend([reset, update, candidate, 0.0]);
                    }
                }
                for channel in 0..hidden {
                    let [reset, update, candidate] = layers
                        .each_ref()
                        .map(|layer| layer.bias().map_or(0.0, |bias| bias[channel]));
                    packed.extend([reset, update, candidate, 0.0]);
                }
            }
            packed.into()
        })
    }

    fn check_state(&self, state: &GruState) -> Result<(), MatrixError> {
        check_length(
            "GRU hidden state",
            self.output_channels(),
            state.hidden.len(),
        )
    }

    fn step_validated(&self, input: &[f32], state: &mut GruState) -> Result<(), RecurrentError> {
        self.layers[0].execute_frames(input, &mut state.scratch, 1)?;
        self.layers[3].execute_accumulate(&state.hidden, &mut state.scratch)?;
        sigmoid(&state.scratch, &mut state.reset)?;

        self.layers[1].execute_frames(input, &mut state.scratch, 1)?;
        self.layers[4].execute_accumulate(&state.hidden, &mut state.scratch)?;
        sigmoid(&state.scratch, &mut state.update)?;

        // Reset is applied AFTER the recurrent candidate affine, including its
        // bias, not to h before the matrix multiply.
        self.layers[5].execute_frames(&state.hidden, &mut state.candidate, 1)?;
        for (candidate, reset) in state.candidate.iter_mut().zip(&state.reset) {
            *candidate *= reset;
        }
        self.layers[2].execute_accumulate(input, &mut state.candidate)?;
        tanh::apply(&state.candidate, &mut state.scratch);

        for ((hidden, update), candidate) in state
            .hidden
            .iter_mut()
            .zip(&state.update)
            .zip(&state.scratch)
        {
            // Keep this multiply, subtraction, multiply, add sequence for stable rounding.
            *hidden *= update;
            let remaining = 1.0 - update;
            let contribution = candidate * remaining;
            *hidden += contribution;
        }
        Ok(())
    }
}

fn sigmoid(input: &[f32], output: &mut [f32]) -> Result<(), InferenceError> {
    #[cfg(target_arch = "x86_64")]
    {
        ActivationOperator::Sigmoid.execute_baseline(input, output)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        ActivationOperator::Sigmoid.execute_reference(input, output)
    }
}

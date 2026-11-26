//! Stateful row queue for temporal convolution.

use super::{Convolution, ConvolutionError, MatrixError};
use crate::synthesis::inference::gpu::Resident;
use std::ops::Range;

/// Stream state is independent of immutable weights and can be cloned for an
/// incremental rendering checkpoint. Only unconsumed input rows are retained.
#[derive(Clone)]
pub struct ConvolutionStream {
    operator: Convolution,
    pending: Pending,
    finished: bool,
}

#[derive(Clone)]
pub(in crate::synthesis::inference) enum Pending {
    Host(Vec<f32>),
    Device(Resident),
}

impl Pending {
    pub(in crate::synthesis::inference) fn len(&self) -> usize {
        match self {
            Self::Host(values) => values.len(),
            Self::Device(values) => values.len(),
        }
    }
}

pub(in crate::synthesis::inference) struct ConvolutionChunk {
    pub(in crate::synthesis::inference) frames: usize,
    pub(in crate::synthesis::inference) pending_tail: Range<usize>,
    finished: bool,
    pub(in crate::synthesis::inference) input_tail: Range<usize>,
}

impl ConvolutionStream {
    pub(super) fn new(operator: Convolution) -> Result<Self, ConvolutionError> {
        let length = operator
            .config
            .padding
            .checked_mul(operator.input_channels())
            .ok_or(MatrixError::InvalidShape)?;
        Ok(Self {
            operator,
            pending: Pending::Host(vec![0.0; length]),
            finished: false,
        })
    }

    pub fn reset(&mut self) {
        self.pending = Pending::Host(vec![
            0.0;
            self.operator.config.padding
                * self.operator.input_channels()
        ]);
        self.finished = false;
    }

    pub fn pending_frames(&self) -> usize {
        self.pending.len() / self.operator.input_channels()
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Supply chronological input once; `final_chunk` makes the right padding
    /// available. A finalized stream must be reset before reuse.
    pub fn process(
        &mut self,
        input: &[f32],
        final_chunk: bool,
    ) -> Result<Vec<f32>, ConvolutionError> {
        #[cfg(feature = "synth-profile")]
        let _profile = crate::synthesis::profiling::span("op.convolution");
        let chunk = self.plan(input.len(), final_chunk)?;
        if input.is_empty() && chunk.frames == 0 {
            return Ok(Vec::new());
        }
        if let Pending::Device(resident) = &self.pending {
            self.pending = Pending::Host(resident.read()?);
        }
        let Pending::Host(pending) = &mut self.pending else {
            unreachable!("resident convolution history was materialized");
        };
        if chunk.frames == 0 {
            pending.extend_from_slice(input);
            return Ok(Vec::new());
        }
        let output = if pending.is_empty() {
            let output = self.operator.execute_pending(input, chunk.frames)?;
            pending.extend_from_slice(&input[chunk.input_tail]);
            output
        } else {
            let previous_length = pending.len();
            pending.extend_from_slice(input);
            let output = self
                .operator
                .execute_pending(pending, chunk.frames)
                .inspect_err(|_| pending.truncate(previous_length))?;
            let consumed = chunk.pending_tail.start + chunk.input_tail.start;
            pending.drain(..consumed);
            output
        };
        if chunk.finished {
            *pending = Vec::new();
        }
        self.finished = chunk.finished;
        Ok(output)
    }

    pub(in crate::synthesis::inference) fn plan(
        &self,
        input_len: usize,
        final_chunk: bool,
    ) -> Result<ConvolutionChunk, ConvolutionError> {
        if self.finished {
            return Err(ConvolutionError::Finished);
        }
        let channels = self.operator.input_channels();
        let input_frames = self.operator.input_frames(input_len)?;
        let available = self
            .pending_frames()
            .checked_add(input_frames)
            .and_then(|frames| {
                frames.checked_add(if final_chunk {
                    self.operator.config.padding
                } else {
                    0
                })
            })
            .ok_or(MatrixError::InvalidShape)?;
        let frames = self.operator.config.available_outputs(available)?;
        frames
            .checked_mul(self.operator.output_channels())
            .ok_or(MatrixError::InvalidShape)?;
        self.pending
            .len()
            .checked_add(input_len)
            .ok_or(MatrixError::InvalidShape)?;
        // A zero-output call leaves the final flag unset.
        let finished = final_chunk && frames != 0;
        let consumed = if finished {
            0
        } else {
            frames * self.operator.config.stride * channels
        };
        let tail_start = if finished {
            input_len
        } else {
            consumed.saturating_sub(self.pending.len())
        };
        let pending_len = self.pending.len();
        let pending_start = if finished {
            pending_len
        } else {
            consumed.min(pending_len)
        };
        Ok(ConvolutionChunk {
            frames,
            pending_tail: pending_start..pending_len,
            finished,
            input_tail: tail_start..input_len,
        })
    }

    pub(in crate::synthesis::inference) fn pending(&self) -> &Pending {
        &self.pending
    }

    pub(in crate::synthesis::inference) fn retain(
        mut self,
        chunk: &ConvolutionChunk,
        pending: Option<Resident>,
    ) -> Self {
        debug_assert_eq!(
            pending.as_ref().map_or(0, Resident::len),
            chunk.pending_tail.len() + chunk.input_tail.len(),
        );
        self.finished = chunk.finished;
        self.pending = pending.map_or_else(|| Pending::Host(Vec::new()), Pending::Device);
        self
    }
}

//! Stateful row queue for temporal convolution.

use super::{Convolution, ConvolutionError, MatrixError};
use std::ops::Range;

/// Stream state is independent of immutable weights and can be cloned for an
/// incremental rendering checkpoint. Only unconsumed input rows are retained.
#[derive(Clone)]
pub struct ConvolutionStream {
    operator: Convolution,
    pending: Vec<f32>,
    finished: bool,
}

pub(in crate::synthesis::inference) struct ConvolutionChunk {
    pub(in crate::synthesis::inference) frames: usize,
    consumed: usize,
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
            pending: vec![0.0; length],
            finished: false,
        })
    }

    pub fn reset(&mut self) {
        self.pending.clear();
        self.pending.resize(
            self.operator.config.padding * self.operator.input_channels(),
            0.0,
        );
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
        let previous_length = self.pending.len();
        self.pending.extend_from_slice(input);
        if chunk.frames == 0 {
            return Ok(Vec::new());
        }
        let output = self
            .operator
            .execute_pending(&self.pending, chunk.frames)
            .inspect_err(|_| {
                self.pending.truncate(previous_length);
            })?;
        self.consume(&chunk);
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
        Ok(ConvolutionChunk {
            frames,
            consumed,
            finished,
            input_tail: tail_start..input_len,
        })
    }

    pub(in crate::synthesis::inference) fn pending(&self) -> &[f32] {
        &self.pending
    }

    pub(in crate::synthesis::inference) fn advance(
        &mut self,
        chunk: &ConvolutionChunk,
        tail: &[f32],
    ) {
        debug_assert_eq!(chunk.input_tail.len(), tail.len());
        self.consume(chunk);
        self.pending.extend_from_slice(tail);
    }

    fn consume(&mut self, chunk: &ConvolutionChunk) {
        if chunk.finished {
            self.pending.clear();
            self.finished = true;
        } else {
            self.pending.drain(..chunk.consumed.min(self.pending.len()));
        }
    }
}

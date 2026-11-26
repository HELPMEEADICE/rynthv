use super::super::{Batch, Resident};
use crate::synthesis::{
    inference::{
        convolution::{Convolution, ConvolutionChunk, ConvolutionStream, Pending},
        matrix::MatrixError,
    },
    voice::{Result, VoiceError},
};
use wgpu::Buffer;

pub(super) struct StreamStep {
    pub(super) node: usize,
    state: ConvolutionStream,
    pub(super) chunk: ConvolutionChunk,
}

impl StreamStep {
    pub(super) fn plan(
        node: usize,
        op: &Convolution,
        state: Option<&ConvolutionStream>,
        input_len: usize,
        flush: bool,
    ) -> Result<Self> {
        let state = match state {
            Some(state) => state.clone(),
            None => op.stream().map_err(error)?,
        };
        let chunk = state.plan(input_len, flush).map_err(error)?;
        Ok(Self { node, state, chunk })
    }

    pub(super) fn lengths(&self) -> [usize; 2] {
        [
            self.state.pending().len(),
            self.chunk.pending_tail.len() + self.chunk.input_tail.len(),
        ]
    }

    pub(super) fn is_resident(&self) -> bool {
        matches!(self.state.pending(), Pending::Device(_))
    }

    pub(super) fn pending_frames(&self) -> usize {
        self.state.pending_frames()
    }

    pub(super) fn previous(
        &self,
        batch: &mut Batch<'_>,
    ) -> std::result::Result<Option<Buffer>, MatrixError> {
        match self.state.pending() {
            Pending::Host(values) if values.is_empty() => Ok(None),
            Pending::Host(values) => Ok(Some(batch.input(values))),
            Pending::Device(values) => batch.resident_input(values).map(Some),
        }
    }

    pub(super) fn retain(
        &self,
        batch: &mut Batch<'_>,
        previous: Option<&Buffer>,
        input: &Buffer,
    ) -> ConvolutionStream {
        let [old, len] = self.lengths();
        let resident = if len == 0 {
            None
        } else if self.chunk.input_tail.is_empty()
            && len == old
            && let Pending::Device(existing) = self.state.pending()
        {
            Some(existing.clone())
        } else {
            Some(self.copy_tail(batch, previous, input, len))
        };
        self.state.clone().retain(&self.chunk, resident)
    }

    fn copy_tail(
        &self,
        batch: &mut Batch<'_>,
        previous: Option<&Buffer>,
        input: &Buffer,
        len: usize,
    ) -> Resident {
        let next = batch.resident(len);
        if let Some(previous) = previous {
            batch.copy_floats(previous, self.chunk.pending_tail.clone(), next.buffer(), 0);
        }
        batch.copy_floats(
            input,
            self.chunk.input_tail.clone(),
            next.buffer(),
            self.chunk.pending_tail.len(),
        );
        next
    }
}

fn error(error: impl std::fmt::Display) -> VoiceError {
    VoiceError::new(error.to_string())
}

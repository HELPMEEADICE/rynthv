use crate::synthesis::{
    inference::convolution::{Convolution, ConvolutionChunk, ConvolutionStream},
    voice::{Result, VoiceError},
};
use std::{collections::HashMap, ops::Range};

pub(super) struct StreamStep {
    pub(super) state: ConvolutionStream,
    pub(super) chunk: ConvolutionChunk,
    pub(super) history: Option<Range<usize>>,
    node: usize,
}

impl StreamStep {
    pub(super) fn plan(
        node: usize,
        op: &Convolution,
        state: Option<&ConvolutionStream>,
        input_len: usize,
        flush: bool,
        device_history_start: Option<usize>,
    ) -> Result<Self> {
        let state = match state {
            Some(state) => state.clone(),
            None => op.stream().map_err(error)?,
        };
        let chunk = state.plan(input_len, flush).map_err(error)?;
        let history = if let Some(start) = device_history_start {
            let end = start
                .checked_add(chunk.input_tail.len())
                .ok_or_else(|| error("GPU convolution history size overflows"))?;
            Some(start..end)
        } else {
            None
        };
        Ok(Self {
            state,
            chunk,
            history,
            node,
        })
    }

    pub(super) fn commit(
        mut self,
        streams: &mut HashMap<usize, ConvolutionStream>,
        input: &[f32],
        history: &[f32],
    ) {
        let tail = match self.history {
            Some(range) => &history[range],
            None => &input[self.chunk.input_tail.clone()],
        };
        self.state.advance(&self.chunk, tail);
        streams.insert(self.node, self.state);
    }
}

fn error(error: impl std::fmt::Display) -> VoiceError {
    VoiceError::new(error.to_string())
}

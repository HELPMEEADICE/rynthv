//! Frame-preserving joins between branches with different streaming latency.

use std::collections::VecDeque;

use crate::synthesis::{
    inference::Sequence,
    voice::{Result, VoiceError},
};

#[derive(Clone, Default)]
pub(crate) struct FrameQueue {
    channels: usize,
    values: VecDeque<f32>,
}

impl FrameQueue {
    pub(crate) fn new(channels: usize) -> Self {
        Self {
            channels,
            values: VecDeque::new(),
        }
    }

    pub(crate) fn frames(&self) -> usize {
        self.values.len().checked_div(self.channels).unwrap_or(0)
    }

    pub(crate) fn push(&mut self, input: Sequence) -> Result<()> {
        self.check_input(&input)?;
        if self.values.is_empty() {
            self.values = input.values.into();
        } else {
            self.values
                .try_reserve(input.values.len())
                .map_err(|_| VoiceError::new("Could not allocate queued frames"))?;
            self.values.extend(input.values);
        }
        Ok(())
    }

    pub(crate) fn push_borrowed(&mut self, input: &Sequence) -> Result<()> {
        self.check_input(input)?;
        self.values
            .try_reserve(input.values.len())
            .map_err(|_| VoiceError::new("Could not allocate queued frames"))?;
        self.values.extend(&input.values);
        Ok(())
    }

    fn check_input(&mut self, input: &Sequence) -> Result<()> {
        input.validate()?;
        // Some residual branches acquire their width from their first shaped batch.
        if self.channels == 0 {
            self.channels = input.channels;
        }
        if input.channels != self.channels {
            return Err(VoiceError::new("Frame queue channel count changed"));
        }
        Ok(())
    }

    pub(crate) fn take(&mut self, frames: usize) -> Result<Sequence> {
        if frames > self.frames() {
            return Err(VoiceError::new("Frame queue has insufficient frames"));
        }
        let count = frames * self.channels;
        let values = if count == self.values.len() {
            std::mem::take(&mut self.values).into()
        } else if count > self.values.len() / 2 {
            // Hand off the large prefix and keep only the delayed tail.
            let tail = self.values.split_off(count);
            std::mem::replace(&mut self.values, tail).into()
        } else {
            self.values.drain(..count).collect()
        };
        Sequence::new(frames, self.channels, values)
    }

    pub(crate) fn ensure_empty(&self, name: &str) -> Result<()> {
        if self.frames() != 0 {
            return Err(VoiceError::new(format!(
                "Final {name} has {} unpaired rows",
                self.frames()
            )));
        }
        Ok(())
    }
}

/// Only explicit stream finalization requires both upstream tails to agree.
pub(crate) fn paired(
    first: &mut FrameQueue,
    second: &mut FrameQueue,
    end: bool,
) -> Result<(Sequence, Sequence)> {
    if end && first.frames() != second.frames() {
        return Err(VoiceError::new(
            "Stream branches ended with unaligned frames",
        ));
    }
    let frames = first.frames().min(second.frames());
    Ok((first.take(frames)?, second.take(frames)?))
}

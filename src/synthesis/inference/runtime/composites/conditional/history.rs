use crate::synthesis::{
    inference::Sequence,
    voice::{Result, VoiceError},
};
use std::ops::Range;

/// Raw input halos for a length-preserving conditional graph. Intermediate
/// activations are recomputed on GPU within each bounded window. CPU history
/// is small and cloneable, so seeking and checkpointing need no device copies.
#[derive(Clone, Default)]
pub(super) struct History {
    input: Vec<f32>,
    condition: Vec<f32>,
    origin: usize,
    emitted: usize,
}

impl History {
    pub(super) fn try_process<T>(
        &mut self,
        input: &Sequence,
        condition: Option<&Sequence>,
        flush: bool,
        radii: [usize; 2],
        render: impl FnOnce(&Sequence, Option<&Sequence>, Range<usize>) -> Result<Option<T>>,
    ) -> Result<Option<T>> {
        self.input.extend_from_slice(&input.values);
        let input_frames = self.input.len() / input.channels;
        let mut stable = input_frames.saturating_sub(radii[0]);
        let mut window_frames = input_frames;
        if let Some(condition) = condition {
            self.condition.extend_from_slice(&condition.values);
            let condition_frames = self.condition.len() / condition.channels;
            stable = stable.min(condition_frames.saturating_sub(radii[1]));
            window_frames = window_frames.max(condition_frames);
        }
        if flush {
            stable = input_frames;
        }
        let start = self.emitted - self.origin;
        if start >= stable {
            return render(input, condition, 0..0);
        }
        // The faster branch may have arrived earlier. Its absent counterpart
        // is padded only outside the published interval's dependency radius.
        let sequence = window(&self.input, input.channels, window_frames)?;
        let conditioning = condition
            .map(|c| window(&self.condition, c.channels, window_frames))
            .transpose()?;
        let Some(output) = render(&sequence, conditioning.as_ref(), start..stable)? else {
            return Ok(None);
        };
        self.emitted = self
            .origin
            .checked_add(stable)
            .ok_or_else(|| VoiceError::new("Conditional stream frame count overflow"))?;
        if !flush {
            let radius = radii[0].max(radii[1]);
            let retained = self.emitted.saturating_sub(radius).max(self.origin);
            let remove = retained - self.origin;
            self.input.drain(..remove * input.channels);
            if let Some(condition) = condition {
                self.condition.drain(..remove * condition.channels);
            }
            self.origin = retained;
        }
        Ok(Some(output))
    }
}

fn window(values: &[f32], channels: usize, frames: usize) -> Result<Sequence> {
    let mut output = Sequence::zeros(frames, channels)?;
    output.values[..values.len()].copy_from_slice(values);
    Ok(output)
}

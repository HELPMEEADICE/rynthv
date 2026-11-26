//! Bounded-lag frame FIFOs. Consumed rows never participate in later batches.

use crate::synthesis::{
    inference::{Sequence, frame_queue::FrameQueue},
    voice::{Result, VoiceError},
};

/// Preserve `left + right` order while projections with distinct receptive
/// fields make different numbers of rows ready on each invocation.
#[derive(Clone, Default)]
pub(super) struct SumQueue {
    left: FrameQueue,
    right: FrameQueue,
}

impl SumQueue {
    pub(super) fn process(
        &mut self,
        left: Sequence,
        right: Sequence,
        flush: bool,
        name: &str,
    ) -> Result<Sequence> {
        if left.channels != right.channels {
            return Err(VoiceError::new(format!("{name} branch widths differ")));
        }
        self.left.push(left)?;
        self.right.push(right)?;
        let frames = self.left.frames().min(self.right.frames());
        let mut output = self.left.take(frames)?;
        let right = self.right.take(frames)?;
        if frames != 0 {
            crate::synthesis::inference::compute::record_cpu(
                "streamed_residual_sum",
                "CPU queue alignment",
            );
        }
        for (target, value) in output.values.iter_mut().zip(right.values) {
            *target += value;
        }
        if flush {
            self.left.ensure_empty(name)?;
            self.right.ensure_empty(name)?;
        }
        Ok(output)
    }
}

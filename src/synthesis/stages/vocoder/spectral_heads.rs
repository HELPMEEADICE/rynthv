//! Parallel vocoder spectral heads and linear overlap between frequency bands.
//!
//! Each branch predicts all heads in contiguous channel blocks. There is no
//! sigmoid, exponential, or normalization at this boundary.

use std::sync::Arc;

use crate::synthesis::{
    execution::checkpoint,
    inference::{
        ModelRuntime, Sequence,
        ops::{MODULE_LIST_TYPE_ID, SEQUENTIAL_TYPE_ID},
    },
    voice::{PreparedTree, Result, VoiceError},
};

pub const SPECTRAL_HEADS_TYPE_ID: [u8; 16] = [
    0x2d, 0xae, 0xaa, 0xac, 0x38, 0xce, 0x69, 0x8d, 0xc6, 0x0c, 0xa1, 0xde, 0x32, 0x53, 0xcb, 0xd5,
];

#[derive(Debug)]
struct BandSpec {
    node: usize,
    start: usize,
    end: usize,
    weights: Arc<[f32]>,
}

#[derive(Debug)]
struct SpectralHeadsSpec {
    head_count: usize,
    channels: usize,
    bands: Vec<BandSpec>,
}

#[derive(Clone, Debug)]
struct BandQueue {
    values: Vec<f32>,
    head: usize,
}

impl BandQueue {
    fn new() -> Self {
        Self {
            values: Vec::new(),
            head: 0,
        }
    }

    fn available(&self) -> usize {
        self.values.len() - self.head
    }

    fn extend(&mut self, values: &[f32]) {
        self.values.extend_from_slice(values);
    }

    fn clear(&mut self) {
        self.values.clear();
        self.head = 0;
    }

    fn compact(&mut self) {
        if self.head >= self.values.len() {
            self.values.clear();
            self.head = 0;
        } else if self.head >= 64 * 1024 {
            self.values.drain(..self.head);
            self.head = 0;
        }
    }
}

#[derive(Clone, Debug)]
pub struct SpectralHeads {
    spec: Arc<SpectralHeadsSpec>,
    queues: Vec<BandQueue>,
}

impl SpectralHeads {
    pub fn prepare(tree: &PreparedTree, index: usize) -> Result<Self> {
        #[cfg(feature = "synth-profile")]
        let _profile = crate::synthesis::profiling::span("vocoder.spectral_heads.prepare");
        let node = tree.node(index)?;
        let [branches] = node.children.as_slice() else {
            return Err(VoiceError::new(
                "Vocoder spectral heads require one module list",
            ));
        };
        if node.type_id != SPECTRAL_HEADS_TYPE_ID {
            return Err(VoiceError::new("Invalid vocoder spectral-head type"));
        }
        let payload = tree.payload(index)?;
        if payload.len() < 8 || !payload.len().is_multiple_of(4) {
            return Err(VoiceError::new("Invalid spectral-head configuration size"));
        }
        let integers = payload
            .as_chunks::<4>()
            .0
            .iter()
            .map(|value| i32::from_le_bytes([value[0], value[1], value[2], value[3]]))
            .collect::<Vec<_>>();
        let head_count = integers[0];
        let band_count = integers[1];
        if !(1..=4).contains(&head_count)
            || band_count < 1
            || usize::try_from(band_count)
                .ok()
                .and_then(|n| n.checked_mul(4))
                .and_then(|n| n.checked_add(2))
                != Some(integers.len())
        {
            return Err(VoiceError::new("Invalid spectral-head or band count"));
        }
        let branches = tree.node(*branches)?;
        if branches.type_id != MODULE_LIST_TYPE_ID || branches.children.len() != band_count as usize
        {
            return Err(VoiceError::new(
                "Spectral-head branches do not match configuration",
            ));
        }
        let mut channels = 0;
        let mut bands = Vec::with_capacity(band_count as usize);
        for (&node, config) in branches
            .children
            .iter()
            .zip(integers[2..].as_chunks::<4>().0)
        {
            let [start, end, fade_in, fade_out] = [config[0], config[1], config[2], config[3]];
            if start < 0
                || end <= start
                || fade_in < 0
                || fade_out < 0
                || fade_in == 1
                || fade_out == 1
                || i64::from(fade_in) + i64::from(fade_out) > i64::from(end - start)
                || tree.node(node)?.type_id != SEQUENTIAL_TYPE_ID
            {
                return Err(VoiceError::new(
                    "Invalid spectral frequency band or branch type",
                ));
            }
            let start = start as usize;
            let end = end as usize;
            let width = end - start;
            let fade_in = fade_in as usize;
            let fade_out = fade_out as usize;
            let weights: Arc<[f32]> = (0..width)
                .map(|bin| {
                    if bin < fade_in {
                        bin as f32 / (fade_in - 1) as f32
                    } else if bin >= width - fade_out {
                        (width - 1 - bin) as f32 / (fade_out - 1) as f32
                    } else {
                        1.0
                    }
                })
                .collect();
            channels = channels.max(end);
            bands.push(BandSpec {
                node,
                start,
                end,
                weights,
            });
        }
        let queues = vec![BandQueue::new(); bands.len()];
        Ok(Self {
            spec: Arc::new(SpectralHeadsSpec {
                head_count: head_count as usize,
                channels,
                bands,
            }),
            queues,
        })
    }

    pub fn head_count(&self) -> usize {
        self.spec.head_count
    }

    pub fn channels(&self) -> usize {
        self.spec.channels
    }

    /// Clear these output queues together with the associated model runtime.
    /// Calling only one reset would combine frames from unrelated segments.
    pub fn reset(&mut self) {
        for queue in &mut self.queues {
            queue.clear();
        }
    }

    /// Return one frame-major frequency sequence per head, in serialized order.
    /// Branch outputs are queued across chunks. The common fresh output count
    /// controls each emitted chunk.
    pub fn execute(
        &mut self,
        runtime: &mut ModelRuntime<'_>,
        input: &Sequence,
        flush: bool,
    ) -> Result<Vec<Sequence>> {
        #[cfg(feature = "synth-profile")]
        let _profile = crate::synthesis::profiling::span("vocoder.spectral_heads.execute");
        input.validate()?;
        let mut frames = usize::MAX;
        for (band, queue) in self.spec.bands.iter().zip(&mut self.queues) {
            let result = runtime.execute(band.node, input, flush)?;
            result.validate()?;
            let expected = (band.end - band.start)
                .checked_mul(self.spec.head_count)
                .ok_or_else(|| VoiceError::new("Spectral band channels overflow"))?;
            if result.channels != expected {
                return Err(VoiceError::new(
                    "Spectral branch output has the wrong channel count",
                ));
            }
            if result.values.iter().any(|value| !value.is_finite()) {
                return Err(VoiceError::new(
                    "Spectral branch produced non-finite values",
                ));
            }
            queue.extend(&result.values);
            frames = frames.min(queue.available() / expected);
        }
        let mut output = (0..self.spec.head_count)
            .map(|_| Sequence::zeros(frames, self.spec.channels))
            .collect::<Result<Vec<_>>>()?;
        for frame in 0..frames {
            checkpoint()?;
            let frame_offset = frame * self.spec.channels;
            for (band, queue) in self.spec.bands.iter().zip(&mut self.queues) {
                let width = band.end - band.start;
                for head in &mut output {
                    let source = &queue.values[queue.head..queue.head + width];
                    let dest = &mut head.values[frame_offset + band.start..][..width];
                    for (target, (&value, &weight)) in
                        dest.iter_mut().zip(source.iter().zip(&*band.weights))
                    {
                        *target += value * weight;
                    }
                    queue.head += width;
                }
            }
        }
        for queue in &mut self.queues {
            queue.compact();
        }
        if flush && self.queues.iter().any(|queue| queue.available() != 0) {
            return Err(VoiceError::new(
                "Spectral branches ended with unaligned frame counts",
            ));
        }
        Ok(output)
    }
}

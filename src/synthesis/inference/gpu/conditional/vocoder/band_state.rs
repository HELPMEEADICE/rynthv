use crate::synthesis::{
    inference::gpu::{Batch, Resident},
    voice::{Result, VoiceError},
};
use wgpu::Buffer;

/// Rows that a faster band has finished while another band still needs context.
#[derive(Clone, Debug)]
pub(crate) struct VocoderBandState {
    pub(super) pending: Vec<Option<Resident>>,
}

impl VocoderBandState {
    pub(crate) fn new(bands: usize) -> Self {
        Self {
            pending: vec![None; bands],
        }
    }

    pub(crate) fn clear(&mut self) {
        self.pending.fill(None);
    }

    pub(crate) fn materialize(&mut self, mut enqueue: impl FnMut(usize, &[f32])) -> Result<()> {
        for (index, pending) in self.pending.iter_mut().enumerate() {
            if let Some(resident) = pending {
                let values = resident
                    .read()
                    .map_err(|error| VoiceError::new(error.to_string()))?;
                enqueue(index, &values);
                *pending = None;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(super) struct BandRows {
    pub(super) pending: usize,
    pub(super) fresh: usize,
    pub(super) width: usize,
}

impl BandRows {
    pub(super) fn ready(self) -> usize {
        self.pending + self.fresh
    }

    pub(super) fn split(self, emitted: usize) -> (usize, usize) {
        let from_pending = emitted.min(self.pending);
        (from_pending, emitted - from_pending)
    }

    pub(super) fn retain(
        self,
        batch: &mut Batch<'_>,
        pending: Option<(&Resident, &Buffer)>,
        fresh: &Buffer,
        emitted: usize,
    ) -> Option<Resident> {
        let remaining = self.ready() - emitted;
        if remaining == 0 {
            return None;
        }
        if emitted == 0 && self.fresh == 0 {
            return pending.map(|(resident, _)| resident.clone());
        }
        let tail = batch.resident(remaining * self.width);
        let (old, new) = self.split(emitted);
        if let Some((_, previous)) = pending {
            batch.copy_floats(
                previous,
                old * self.width..self.pending * self.width,
                tail.buffer(),
                0,
            );
        }
        batch.copy_floats(
            fresh,
            new * self.width..self.fresh * self.width,
            tail.buffer(),
            (self.pending - old) * self.width,
        );
        Some(tail)
    }
}

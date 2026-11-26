//! Immutable device snapshots shared by cloned stream checkpoints.

use super::{Batch, Device};
use crate::synthesis::inference::{compute, matrix::MatrixError};
use std::{fmt, sync::Arc};
use wgpu::{Buffer, BufferUsages};

#[derive(Clone)]
pub(in super::super) struct Resident {
    device: Arc<Device>,
    buffer: Buffer,
    len: usize,
}

impl Resident {
    pub(super) fn new(device: Arc<Device>, len: usize) -> Self {
        let buffer = device.raw.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RynthV inference stream state"),
            size: len as u64 * 4,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            device,
            buffer,
            len,
        }
    }

    pub(super) fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    pub(super) fn on(&self, device: &Arc<Device>) -> bool {
        Arc::ptr_eq(&self.device, device)
    }

    pub(in super::super) fn len(&self) -> usize {
        self.len
    }

    pub(in super::super) fn read(&self) -> Result<Vec<f32>, MatrixError> {
        let mut output = vec![0.0; self.len];
        let mut batch = Batch::new(&self.device, false)?;
        let result = batch.read(&self.buffer, &mut output);
        compute::record_readback(&self.device, &batch);
        result.map(|()| output)
    }
}

impl fmt::Debug for Resident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Resident")
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

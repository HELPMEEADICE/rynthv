mod readback;

use super::{Device, Resident, timestamps::Timestamps};
use crate::synthesis::{
    execution,
    inference::{compute::Activity, matrix::MatrixError},
};
use std::{ops::Range, sync::Arc};
use wgpu::{Buffer, BufferUsages};

#[derive(Clone, Copy)]
pub(super) struct Kernel {
    pub entry: &'static str,
    pub source: &'static str,
}

struct Dispatch {
    pipeline: wgpu::ComputePipeline,
    bindings: wgpu::BindGroup,
    grid: [u32; 2],
}

pub(in super::super) struct Batch<'a> {
    device: &'a Arc<Device>,
    encoder: Option<wgpu::CommandEncoder>,
    buffers: Vec<Buffer>,
    dispatches: Vec<Dispatch>,
    activity: Activity,
    timestamps: Option<Timestamps>,
    scratch_bytes: u64,
}

impl<'a> Batch<'a> {
    pub(in super::super) fn new(
        device: &'a Arc<Device>,
        timestamps: bool,
    ) -> Result<Self, MatrixError> {
        checkpoint(device)?;
        Ok(Self {
            device,
            encoder: Some(
                device
                    .raw
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("RynthV inference"),
                    }),
            ),
            buffers: Vec::new(),
            dispatches: Vec::new(),
            activity: Activity::default(),
            timestamps: timestamps.then(|| Timestamps::new(device)).flatten(),
            scratch_bytes: 0,
        })
    }

    pub(in super::super) fn activity(&self) -> &Activity {
        &self.activity
    }

    pub(super) fn input(&mut self, values: &[f32]) -> Buffer {
        self.upload(
            bytemuck::cast_slice(values),
            BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        )
    }

    pub(super) fn staging_input(&mut self, len: usize) -> Buffer {
        self.buffer(
            len as u64 * 4,
            BufferUsages::STORAGE | BufferUsages::COPY_DST,
        )
    }

    pub(super) fn write_slice(&mut self, buffer: &Buffer, offset_floats: usize, values: &[f32]) {
        if values.is_empty() {
            return;
        }
        let bytes = bytemuck::cast_slice(values);
        self.device
            .queue
            .write_buffer(buffer, offset_floats as u64 * 4, bytes);
        self.activity.upload_count += 1;
        self.activity.upload_bytes += bytes.len() as u64;
    }

    pub(super) fn copy_floats(
        &mut self,
        source: &Buffer,
        range: Range<usize>,
        destination: &Buffer,
        dst_offset_floats: usize,
    ) {
        if range.is_empty() {
            return;
        }
        self.flush_dispatches();
        self.encoder.as_mut().unwrap().copy_buffer_to_buffer(
            source,
            range.start as u64 * 4,
            destination,
            dst_offset_floats as u64 * 4,
            range.len() as u64 * 4,
        );
    }

    pub(super) fn output(&mut self, len: usize) -> Buffer {
        self.buffer(
            len as u64 * 4,
            BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        )
    }

    pub(super) fn cleared_output(&mut self, len: usize) -> Buffer {
        let buffer = self.buffer(
            len as u64 * 4,
            BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
        );
        self.flush_dispatches();
        self.encoder
            .as_mut()
            .unwrap()
            .clear_buffer(&buffer, 0, None);
        buffer
    }

    pub(super) fn resident(&mut self, len: usize) -> Resident {
        let resident = Resident::new(self.device.clone(), len);
        self.track_buffer(resident.buffer());
        resident
    }

    pub(super) fn resident_input(&mut self, resident: &Resident) -> Result<Buffer, MatrixError> {
        if let Some(buffer) = self.current_resident(resident) {
            Ok(buffer.clone())
        } else {
            Ok(self.input(&resident.read()?))
        }
    }

    pub(super) fn current_resident<'b>(&self, resident: &'b Resident) -> Option<&'b Buffer> {
        resident.on(self.device).then(|| resident.buffer())
    }

    pub(super) fn validation_index(&mut self) -> Buffer {
        self.upload(
            bytemuck::bytes_of(&u32::MAX),
            BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        )
    }

    pub(super) fn weight(&mut self, source: &Arc<[f32]>) -> Buffer {
        self.device
            .cache
            .lock()
            .unwrap()
            .weight(self.device, source, &mut self.activity)
    }

    pub(super) fn grid(&self, groups: u32) -> Result<[u32; 2], MatrixError> {
        let limit = self.device.limits.max_compute_workgroups_per_dimension;
        let x = groups.min(limit);
        let y = groups.div_ceil(x);
        if y > limit {
            return Err(MatrixError::Compute(
                "WebGPU dispatch exceeds device workgroup limits".into(),
            ));
        }
        Ok([x, y])
    }

    pub(super) fn dispatch(
        &mut self,
        kernel: Kernel,
        grid: [u32; 2],
        params: [u32; 16],
        bindings: [Option<&Buffer>; 5],
    ) -> Result<(), MatrixError> {
        checkpoint(self.device)?;
        let (pipeline, layout) =
            self.device
                .cache
                .lock()
                .unwrap()
                .pipeline(self.device, kernel, &mut self.activity);
        let params = self.upload(bytemuck::bytes_of(&params), BufferUsages::UNIFORM);
        let mut entries: smallvec::SmallVec<[_; 6]> = [0, 1, 2, 3, 5]
            .into_iter()
            .zip(bindings)
            .filter_map(|(binding, buffer)| {
                Some(wgpu::BindGroupEntry {
                    binding,
                    resource: buffer?.as_entire_binding(),
                })
            })
            .collect();
        entries.push(wgpu::BindGroupEntry {
            binding: 4,
            resource: params.as_entire_binding(),
        });
        checkpoint(self.device)?;
        let bind_group = self
            .device
            .raw
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(kernel.entry),
                layout: &layout,
                entries: &entries,
            });
        if self.timestamps.is_some() {
            self.flush_dispatches();
        }
        let timestamp_writes = self
            .timestamps
            .as_mut()
            .and_then(|timestamps| timestamps.mark(kernel.entry));
        if let Some(timestamp_writes) = timestamp_writes {
            let mut pass =
                self.encoder
                    .as_mut()
                    .unwrap()
                    .begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some(kernel.entry),
                        timestamp_writes: Some(timestamp_writes),
                    });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(grid[0], grid[1], 1);
        } else {
            self.activity.untimed_dispatches += 1;
            self.dispatches.push(Dispatch {
                pipeline,
                bindings: bind_group,
                grid,
            });
        }
        *self
            .activity
            .gpu_dispatches
            .entry(kernel.entry)
            .or_default() += 1;
        Ok(())
    }

    fn flush_dispatches(&mut self) {
        if self.dispatches.is_empty() {
            return;
        }
        let mut pass =
            self.encoder
                .as_mut()
                .unwrap()
                .begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("RynthV inference"),
                    timestamp_writes: None,
                });
        for dispatch in self.dispatches.drain(..) {
            pass.set_pipeline(&dispatch.pipeline);
            pass.set_bind_group(0, &dispatch.bindings, &[]);
            pass.dispatch_workgroups(dispatch.grid[0], dispatch.grid[1], 1);
        }
    }

    fn upload(&mut self, contents: &[u8], usage: BufferUsages) -> Buffer {
        let buffer = self.buffer(contents.len() as u64, usage | BufferUsages::COPY_DST);
        if !contents.is_empty() {
            self.device.queue.write_buffer(&buffer, 0, contents);
            self.activity.upload_count += 1;
            self.activity.upload_bytes += contents.len() as u64;
        }
        buffer
    }

    fn buffer(&mut self, bytes: u64, usage: BufferUsages) -> Buffer {
        let buffer = self
            .device
            .cache
            .lock()
            .unwrap()
            .buffer(self.device, bytes, usage);
        self.buffers.push(buffer.clone());
        self.track_buffer(&buffer);
        buffer
    }

    fn track_buffer(&mut self, buffer: &Buffer) {
        self.scratch_bytes += buffer.size();
        self.activity.peak_batch_buffer_bytes = self
            .activity
            .peak_batch_buffer_bytes
            .max(self.scratch_bytes);
    }
}

fn checkpoint(device: &Device) -> Result<(), MatrixError> {
    execution::checkpoint().map_err(|_| MatrixError::Cancelled)?;
    device
        .error()
        .map_or(Ok(()), |error| Err(MatrixError::Compute(error)))
}

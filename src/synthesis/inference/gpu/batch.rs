use super::{Device, timestamps::Timestamps};
use crate::synthesis::{
    execution,
    inference::{compute::Activity, matrix::MatrixError},
};
use std::{
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};
use wgpu::{Buffer, BufferUsages};

#[derive(Clone, Copy)]
pub(super) struct Kernel {
    pub entry: &'static str,
    pub source: &'static str,
}

pub(in super::super) struct Batch<'a> {
    device: &'a Device,
    encoder: Option<wgpu::CommandEncoder>,
    buffers: Vec<Buffer>,
    activity: Activity,
    timestamps: Option<Timestamps>,
    scratch_bytes: u64,
}

impl<'a> Batch<'a> {
    pub(in super::super) fn new(device: &'a Device, timestamps: bool) -> Result<Self, MatrixError> {
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
            activity: Activity::default(),
            timestamps: timestamps.then(|| Timestamps::new(device)).flatten(),
            scratch_bytes: 0,
        })
    }

    pub(in super::super) fn activity(&self) -> &Activity {
        &self.activity
    }

    pub(super) fn input(&mut self, values: &[f32]) -> Buffer {
        self.upload(bytemuck::cast_slice(values), BufferUsages::STORAGE)
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
        destination: &Buffer,
        dst_offset_floats: usize,
        len_floats: usize,
    ) {
        if len_floats == 0 {
            return;
        }
        self.encoder.as_mut().unwrap().copy_buffer_to_buffer(
            source,
            0,
            destination,
            dst_offset_floats as u64 * 4,
            len_floats as u64 * 4,
        );
    }

    pub(super) fn output(&mut self, len: usize) -> Buffer {
        self.buffer(
            len as u64 * 4,
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
        let pipeline =
            self.device
                .cache
                .lock()
                .unwrap()
                .pipeline(self.device, kernel, &mut self.activity);
        let params = self.upload(bytemuck::bytes_of(&params), BufferUsages::UNIFORM);
        let mut entries: Vec<_> = [0, 1, 2, 3, 5]
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
                layout: &pipeline.get_bind_group_layout(0),
                entries: &entries,
            });
        let timestamp_writes = self
            .timestamps
            .as_mut()
            .and_then(|timestamps| timestamps.mark(kernel.entry));
        if timestamp_writes.is_none() {
            self.activity.untimed_dispatches += 1;
        }
        let mut pass =
            self.encoder
                .as_mut()
                .unwrap()
                .begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some(kernel.entry),
                    timestamp_writes,
                });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(grid[0], grid[1], 1);
        *self
            .activity
            .gpu_dispatches
            .entry(kernel.entry)
            .or_default() += 1;
        Ok(())
    }

    pub(super) fn read(&mut self, source: &Buffer, output: &mut [f32]) -> Result<(), MatrixError> {
        self.read_from(source, 0, output)
    }

    pub(super) fn read_from(
        &mut self,
        source: &Buffer,
        first: usize,
        output: &mut [f32],
    ) -> Result<(), MatrixError> {
        checkpoint(self.device)?;
        let bytes = output.len() as u64 * 4;
        let mut encoder = self.encoder.take().unwrap();
        let offset = bytes.next_multiple_of(8);
        let timestamp_bytes = self.timestamps.as_ref().map_or(0, Timestamps::bytes);
        let mapped_bytes = if timestamp_bytes == 0 {
            bytes
        } else {
            offset + timestamp_bytes
        };
        let staging = self.buffer(
            mapped_bytes,
            BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        );
        encoder.copy_buffer_to_buffer(source, first as u64 * 4, &staging, 0, Some(bytes));
        if timestamp_bytes != 0 {
            let resolved = self.buffer(
                timestamp_bytes,
                BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC,
            );
            self.timestamps
                .as_ref()
                .unwrap()
                .resolve(&mut encoder, &resolved, &staging, offset);
        }
        let started = Instant::now();
        let submission = self.device.queue.submit([encoder.finish()]);
        self.activity.gpu_submissions += 1;
        self.activity.readback_count += 1;
        self.activity.readback_bytes += mapped_bytes;
        let slice = staging.slice(..mapped_bytes);
        let (tx, rx) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |status| {
            let _ = tx.send(status);
        });
        let mapped = loop {
            checkpoint(self.device)?;
            match self.device.raw.poll(wgpu::PollType::Wait {
                submission_index: Some(submission.clone()),
                timeout: Some(Duration::from_millis(5)),
            }) {
                Ok(_) => {}
                Err(wgpu::PollError::Timeout) => {}
                Err(error) => return Err(MatrixError::Compute(error.to_string())),
            }
            // A concurrent poll can collect the callback and deliver it after
            // this thread's wait returns.
            match rx.recv_timeout(Duration::from_millis(5)) {
                Ok(status) => break status,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => {
                    return Err(MatrixError::Compute(format!(
                        "WebGPU readback callback: {error}"
                    )));
                }
            }
        };
        checkpoint(self.device)?;
        mapped.map_err(|e| MatrixError::Compute(format!("WebGPU readback: {e}")))?;
        {
            let data = slice
                .get_mapped_range()
                .map_err(|e| MatrixError::Compute(format!("WebGPU mapped range: {e}")))?;
            let values: &[f32] = bytemuck::cast_slice(&data[..bytes as usize]);
            if values.iter().any(|value| !value.is_finite()) {
                return Err(MatrixError::Compute(
                    "WebGPU inference produced a nonfinite value".into(),
                ));
            }
            output.copy_from_slice(values);
            if timestamp_bytes != 0 {
                self.timestamps.as_ref().unwrap().record(
                    &data[offset as usize..],
                    self.device.queue.get_timestamp_period(),
                    &mut self.activity,
                );
            }
        }
        staging.unmap();
        self.activity.completed_gpu_submissions += 1;
        self.activity.readback_ms += started.elapsed().as_secs_f64() * 1000.0;
        let mut cache = self.device.cache.lock().unwrap();
        for buffer in self.buffers.drain(..) {
            cache.recycle(buffer);
        }
        Ok(())
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
        self.scratch_bytes += buffer.size();
        self.activity.peak_batch_buffer_bytes = self
            .activity
            .peak_batch_buffer_bytes
            .max(self.scratch_bytes);
        buffer
    }
}

fn checkpoint(device: &Device) -> Result<(), MatrixError> {
    execution::checkpoint().map_err(|_| MatrixError::Cancelled)?;
    device
        .error()
        .map_or(Ok(()), |error| Err(MatrixError::Compute(error)))
}

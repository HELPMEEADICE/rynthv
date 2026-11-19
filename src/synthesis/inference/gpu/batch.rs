use super::Device;
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
}

impl<'a> Batch<'a> {
    pub(in super::super) fn new(device: &'a Device) -> Result<Self, MatrixError> {
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
        })
    }

    pub(in super::super) fn activity(&self) -> &Activity {
        &self.activity
    }

    pub(super) fn input(&mut self, values: &[f32]) -> Buffer {
        self.upload(bytemuck::cast_slice(values), BufferUsages::STORAGE)
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
        let mut pass =
            self.encoder
                .as_mut()
                .unwrap()
                .begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some(kernel.entry),
                    timestamp_writes: None,
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
        checkpoint(self.device)?;
        let bytes = output.len() as u64 * 4;
        let staging = self.buffer(bytes, BufferUsages::MAP_READ | BufferUsages::COPY_DST);
        let mut encoder = self.encoder.take().unwrap();
        encoder.copy_buffer_to_buffer(source, 0, &staging, 0, Some(bytes));
        let started = Instant::now();
        let submission = self.device.queue.submit([encoder.finish()]);
        self.activity.gpu_submissions += 1;
        self.activity.readback_count += 1;
        self.activity.readback_bytes += bytes;
        let slice = staging.slice(..bytes);
        let (tx, rx) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |status| {
            let _ = tx.send(status);
        });
        loop {
            checkpoint(self.device)?;
            match self.device.raw.poll(wgpu::PollType::Wait {
                submission_index: Some(submission.clone()),
                timeout: Some(Duration::from_millis(5)),
            }) {
                Ok(_) => break,
                Err(wgpu::PollError::Timeout) => {}
                Err(error) => return Err(MatrixError::Compute(error.to_string())),
            }
        }
        checkpoint(self.device)?;
        rx.try_recv()
            .map_err(|e| MatrixError::Compute(format!("WebGPU readback callback: {e}")))?
            .map_err(|e| MatrixError::Compute(format!("WebGPU readback: {e}")))?;
        {
            let data = slice
                .get_mapped_range()
                .map_err(|e| MatrixError::Compute(format!("WebGPU mapped range: {e}")))?;
            let values: &[f32] = bytemuck::cast_slice(&data);
            if values.iter().any(|value| !value.is_finite()) {
                return Err(MatrixError::Compute(
                    "WebGPU inference produced a nonfinite value".into(),
                ));
            }
            output.copy_from_slice(values);
        }
        staging.unmap();
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
        buffer
    }
}

fn checkpoint(device: &Device) -> Result<(), MatrixError> {
    execution::checkpoint().map_err(|_| MatrixError::Cancelled)?;
    device
        .error()
        .map_or(Ok(()), |error| Err(MatrixError::Compute(error)))
}

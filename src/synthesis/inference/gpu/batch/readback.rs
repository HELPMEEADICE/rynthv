//! Staging and mapping CPU-visible results from an inference batch.

use super::{Batch, Device, Timestamps, checkpoint};
use crate::synthesis::inference::matrix::MatrixError;
use std::ops::Range;
#[cfg(not(target_arch = "wasm32"))]
use std::{sync::mpsc, time::Duration};
use web_time::Instant;
use wgpu::{Buffer, BufferUsages};

pub(in crate::synthesis::inference::gpu) struct Readback {
    buffer: Buffer,
    output_bytes: u64,
    history_bytes: u64,
    timestamp_offset: u64,
    validation_label: Option<&'static str>,
}

impl Readback {
    pub(in crate::synthesis::inference::gpu) fn copy_history(
        &self,
        batch: &mut Batch<'_>,
        source: &Buffer,
        range: Range<usize>,
        offset: usize,
    ) {
        batch.copy_floats(
            source,
            range,
            &self.buffer,
            (self.output_bytes / 4) as usize + offset,
        );
    }
}

impl Batch<'_> {
    pub(in crate::synthesis::inference::gpu) fn read(
        &mut self,
        source: &Buffer,
        output: &mut [f32],
    ) -> Result<(), MatrixError> {
        self.read_from(source, 0, output)
    }

    pub(in crate::synthesis::inference::gpu) fn read_from(
        &mut self,
        source: &Buffer,
        first: usize,
        output: &mut [f32],
    ) -> Result<(), MatrixError> {
        let staging = self.stage_readback(output.len(), 0, None, 0);
        self.read_staged(staging, source, first, output, &mut [], None)
    }

    pub(in crate::synthesis::inference::gpu) fn stage_readback(
        &mut self,
        output_len: usize,
        history_len: usize,
        validation_label: Option<&'static str>,
        pending_dispatches: usize,
    ) -> Readback {
        let output_bytes = output_len as u64 * 4;
        let history_bytes = history_len as u64 * 4;
        let data_bytes = output_bytes + history_bytes;
        let status_end = data_bytes + if validation_label.is_some() { 4 } else { 0 };
        let timestamp_offset = status_end.next_multiple_of(8);
        // A chain stages its history before all of its dispatches are encoded.
        let timestamp_bytes = self
            .timestamps
            .as_ref()
            .map_or(0, |timestamps| timestamps.bytes_after(pending_dispatches));
        let capacity = if timestamp_bytes == 0 {
            status_end
        } else {
            timestamp_offset + timestamp_bytes
        };
        let buffer = self.buffer(capacity, BufferUsages::MAP_READ | BufferUsages::COPY_DST);
        Readback {
            buffer,
            output_bytes,
            history_bytes,
            timestamp_offset,
            validation_label,
        }
    }

    pub(in crate::synthesis::inference::gpu) fn read_staged(
        &mut self,
        staging: Readback,
        source: &Buffer,
        first: usize,
        output: &mut [f32],
        history: &mut [f32],
        validation_index: Option<&Buffer>,
    ) -> Result<(), MatrixError> {
        checkpoint(self.device)?;
        self.flush_dispatches();
        let Readback {
            buffer,
            output_bytes,
            history_bytes,
            timestamp_offset,
            validation_label,
        } = staging;
        let data_bytes = output_bytes + history_bytes;
        debug_assert_eq!(output_bytes, output.len() as u64 * 4);
        debug_assert_eq!(history_bytes, history.len() as u64 * 4);
        debug_assert_eq!(validation_label.is_some(), validation_index.is_some());
        let mut encoder = self.encoder.take().unwrap();
        if output_bytes != 0 {
            encoder.copy_buffer_to_buffer(source, first as u64 * 4, &buffer, 0, Some(output_bytes));
        }
        if let Some(validation_index) = validation_index {
            encoder.copy_buffer_to_buffer(validation_index, 0, &buffer, data_bytes, Some(4));
        }
        let timestamp_bytes = self.timestamps.as_ref().map_or(0, Timestamps::bytes);
        let status_end = data_bytes + if validation_index.is_some() { 4 } else { 0 };
        let mapped_bytes = if timestamp_bytes == 0 {
            status_end
        } else {
            timestamp_offset + timestamp_bytes
        };
        if timestamp_bytes != 0 {
            let resolved = self.buffer(
                timestamp_bytes,
                BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC,
            );
            self.timestamps.as_ref().unwrap().resolve(
                &mut encoder,
                &resolved,
                &buffer,
                timestamp_offset,
            );
        }
        let encoded = encoder.finish();
        let submitting = Instant::now();
        let submission = self.device.queue.submit([encoded]);
        self.activity.submit_ms += submitting.elapsed().as_secs_f64() * 1000.0;
        self.activity.gpu_submissions += 1;
        self.activity.readback_count += 1;
        self.activity.readback_bytes += mapped_bytes;
        let slice = buffer.slice(..mapped_bytes);
        let started = Instant::now();
        map_read(self.device, &slice, submission)?
            .map_err(|e| MatrixError::Compute(format!("WebGPU readback: {e}")))?;
        let read = (|| {
            checkpoint(self.device)?;
            let data = slice
                .get_mapped_range()
                .map_err(|e| MatrixError::Compute(format!("WebGPU mapped range: {e}")))?;
            if let Some(label) = validation_label {
                let offset = data_bytes as usize;
                let invalid = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
                if invalid != u32::MAX {
                    return Err(MatrixError::Compute(format!(
                        "WebGPU inference produced a nonfinite value at element {invalid} {label}"
                    )));
                }
            }
            let values: &[f32] = bytemuck::cast_slice(&data[..output_bytes as usize]);
            if let Some((index, value)) = values.iter().enumerate().find(|(_, v)| !v.is_finite()) {
                let kernels = self
                    .activity
                    .gpu_dispatches
                    .keys()
                    .copied()
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(MatrixError::Compute(format!(
                    "WebGPU inference produced a nonfinite value ({value}) at output element {} in batch [{kernels}]",
                    first + index,
                )));
            }
            output.copy_from_slice(values);
            history.copy_from_slice(bytemuck::cast_slice(
                &data[output_bytes as usize..data_bytes as usize],
            ));
            if timestamp_bytes != 0 {
                self.timestamps.as_ref().unwrap().record(
                    &data[timestamp_offset as usize..],
                    self.device.queue.get_timestamp_period(),
                    &mut self.activity,
                );
            }
            Ok(())
        })();
        buffer.unmap();
        read?;
        self.activity.completed_gpu_submissions += 1;
        self.activity.readback_ms += started.elapsed().as_secs_f64() * 1000.0;
        let mut cache = self.device.cache.lock().unwrap();
        for buffer in self.buffers.drain(..) {
            cache.recycle(buffer);
        }
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn map_read(
    device: &Device,
    slice: &wgpu::BufferSlice<'_>,
    submission: wgpu::SubmissionIndex,
) -> Result<Result<(), wgpu::BufferAsyncError>, MatrixError> {
    let (tx, rx) = mpsc::sync_channel(1);
    slice.map_async(wgpu::MapMode::Read, move |status| {
        let _ = tx.send(status);
    });
    loop {
        checkpoint(device)?;
        match device.raw.poll(wgpu::PollType::Wait {
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
            Ok(status) => return Ok(status),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => {
                return Err(MatrixError::Compute(format!(
                    "WebGPU readback callback: {error}"
                )));
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn map_read(
    _: &Device,
    slice: &wgpu::BufferSlice<'_>,
    _: wgpu::SubmissionIndex,
) -> Result<Result<(), wgpu::BufferAsyncError>, MatrixError> {
    let (sender, receiver) = futures_channel::oneshot::channel();
    slice.map_async(wgpu::MapMode::Read, move |status| {
        let _ = sender.send(status);
    });
    crate::platform::web::wait(receiver)
        .map_err(MatrixError::Compute)?
        .map_err(|error| MatrixError::Compute(format!("WebGPU readback callback: {error}")))
}

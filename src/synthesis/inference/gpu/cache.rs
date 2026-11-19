use super::{batch::Kernel, device::Device};
use crate::synthesis::inference::compute::Activity;
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
    time::Instant,
};
use wgpu::{Buffer, BufferUsages, ComputePipeline, ShaderModule, util::DeviceExt};

const WEIGHT_BUDGET: u64 = 384 * 1024 * 1024;
const BUFFER_BUDGET: u64 = 64 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Cache {
    weights: HashMap<usize, Weight>,
    weight_bytes: u64,
    clock: u64,
    modules: HashMap<&'static str, ShaderModule>,
    pipelines: HashMap<&'static str, ComputePipeline>,
    buffers: HashMap<(BufferUsages, u64), Vec<Buffer>>,
    buffer_bytes: u64,
}

struct Weight {
    source: Weak<[f32]>,
    buffer: Buffer,
    used: u64,
}

impl Cache {
    pub(super) fn weight(
        &mut self,
        device: &Device,
        source: &Arc<[f32]>,
        activity: &mut Activity,
    ) -> Buffer {
        let key = Arc::as_ptr(source).cast::<f32>() as usize;
        self.clock += 1;
        if let Some(weight) = self.weights.get_mut(&key) {
            weight.used = self.clock;
            activity.weight_hits += 1;
            return weight.buffer.clone();
        }
        // The weak allocation reserves the address, including after the CPU data
        // is dropped. Dead entries are discarded when preparing new weights.
        self.weights.retain(|_, weight| {
            let alive = weight.source.strong_count() != 0;
            if !alive {
                self.weight_bytes -= weight.buffer.size();
            }
            alive
        });
        let started = Instant::now();
        let data = bytemuck::cast_slice(source);
        let buffer = device
            .raw
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("RynthV prepared weights"),
                contents: data,
                usage: BufferUsages::STORAGE,
            });
        let size = buffer.size();
        activity.weight_misses += 1;
        activity.weight_upload_bytes += size;
        activity.weight_upload_host_ms += started.elapsed().as_secs_f64() * 1000.0;
        activity.upload_count += 1;
        activity.upload_bytes += size;
        if size <= WEIGHT_BUDGET {
            while size + self.weight_bytes > WEIGHT_BUDGET {
                let oldest = *self
                    .weights
                    .iter()
                    .min_by_key(|(_, weight)| weight.used)
                    .unwrap()
                    .0;
                self.weight_bytes -= self.weights.remove(&oldest).unwrap().buffer.size();
            }
            self.weights.insert(
                key,
                Weight {
                    source: Arc::downgrade(source),
                    buffer: buffer.clone(),
                    used: self.clock,
                },
            );
            self.weight_bytes += size;
        }
        buffer
    }

    pub(super) fn pipeline(
        &mut self,
        device: &Device,
        kernel: Kernel,
        activity: &mut Activity,
    ) -> ComputePipeline {
        if let Some(pipeline) = self.pipelines.get(kernel.entry) {
            activity.pipeline_hits += 1;
            return pipeline.clone();
        }
        let module = self.modules.entry(kernel.source).or_insert_with(|| {
            device
                .raw
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(kernel.entry),
                    source: wgpu::ShaderSource::Wgsl(kernel.source.into()),
                })
        });
        let pipeline = device
            .raw
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(kernel.entry),
                layout: None,
                module,
                entry_point: Some(kernel.entry),
                compilation_options: Default::default(),
                cache: None,
            });
        self.pipelines.insert(kernel.entry, pipeline.clone());
        activity.pipeline_misses += 1;
        pipeline
    }

    pub(super) fn buffer(&mut self, device: &Device, bytes: u64, usage: BufferUsages) -> Buffer {
        let mut limit = device.limits.max_buffer_size;
        if usage.contains(BufferUsages::STORAGE) {
            limit = limit.min(device.limits.max_storage_buffer_binding_size);
        }
        let size = bytes.max(4).next_power_of_two().min(limit) & !3;
        let class = (usage, size);
        if let Some(buffer) = self.buffers.get_mut(&class).and_then(Vec::pop) {
            self.buffer_bytes -= size;
            return buffer;
        }
        device.raw.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RynthV inference scratch"),
            size,
            usage,
            mapped_at_creation: false,
        })
    }

    pub(super) fn recycle(&mut self, buffer: Buffer) {
        let size = buffer.size();
        if self.buffer_bytes + size <= BUFFER_BUDGET {
            let buffers = self.buffers.entry((buffer.usage(), size)).or_default();
            if buffers.len() < 8 {
                buffers.push(buffer);
                self.buffer_bytes += size;
            }
        }
    }
}

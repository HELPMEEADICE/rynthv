use super::Device;
use crate::synthesis::inference::compute::Activity;

const MAX_PASSES: usize = 128;

pub(super) struct Timestamps {
    queries: wgpu::QuerySet,
    kernels: Vec<&'static str>,
}

impl Timestamps {
    pub(super) const MAX_BYTES: u64 = MAX_PASSES as u64 * 16;

    pub(super) fn new(device: &Device) -> Option<Self> {
        device
            .raw
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
            .then(|| Self {
                queries: device.raw.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("RynthV compute timestamps"),
                    ty: wgpu::QueryType::Timestamp,
                    count: (MAX_PASSES * 2) as u32,
                }),
                kernels: Vec::new(),
            })
    }

    pub(super) fn mark(
        &mut self,
        kernel: &'static str,
    ) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        let index = self.kernels.len();
        if index == MAX_PASSES {
            return None;
        }
        self.kernels.push(kernel);
        Some(wgpu::ComputePassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some((index * 2) as u32),
            end_of_pass_write_index: Some((index * 2 + 1) as u32),
        })
    }

    pub(super) fn bytes(&self) -> u64 {
        self.bytes_after(0)
    }

    pub(super) fn bytes_after(&self, passes: usize) -> u64 {
        self.kernels.len().saturating_add(passes).min(MAX_PASSES) as u64 * 16
    }

    pub(super) fn resolve(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        resolved: &wgpu::Buffer,
        staging: &wgpu::Buffer,
        offset: u64,
    ) {
        encoder.resolve_query_set(
            &self.queries,
            0..(self.kernels.len() * 2) as u32,
            resolved,
            0,
        );
        encoder.copy_buffer_to_buffer(resolved, 0, staging, offset, Some(self.bytes()));
    }

    pub(super) fn record(&self, data: &[u8], period_ns: f32, activity: &mut Activity) {
        for (&kernel, pair) in self.kernels.iter().zip(data.as_chunks::<16>().0) {
            let first = u64::from_le_bytes(pair[..8].try_into().unwrap());
            let last = u64::from_le_bytes(pair[8..].try_into().unwrap());
            let milliseconds =
                last.saturating_sub(first) as f64 * f64::from(period_ns) / 1_000_000.0;
            *activity.gpu_kernel_ms.entry(kernel).or_default() += milliseconds;
        }
    }
}

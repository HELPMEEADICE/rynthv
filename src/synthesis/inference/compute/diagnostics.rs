use super::Status;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Default, Serialize)]
pub struct Activity {
    pub gpu_submissions: u64,
    pub completed_gpu_submissions: u64,
    pub submit_ms: f64,
    pub gpu_dispatches: BTreeMap<&'static str, u64>,
    pub cpu_operators: BTreeMap<&'static str, BTreeMap<&'static str, u64>>,
    pub upload_count: u64,
    pub upload_bytes: u64,
    pub readback_count: u64,
    pub readback_bytes: u64,
    pub readback_ms: f64,
    pub weight_hits: u64,
    pub weight_misses: u64,
    pub weight_upload_bytes: u64,
    pub weight_upload_host_ms: f64,
    pub pipeline_hits: u64,
    pub pipeline_misses: u64,
    pub pipeline_build_ms: f64,
    pub gpu_kernel_ms: BTreeMap<&'static str, f64>,
    pub untimed_dispatches: u64,
    pub peak_weight_cache_bytes: u64,
    pub peak_batch_buffer_bytes: u64,
}

impl Activity {
    pub(super) fn merge(&mut self, other: &Self) {
        self.gpu_submissions += other.gpu_submissions;
        self.completed_gpu_submissions += other.completed_gpu_submissions;
        self.submit_ms += other.submit_ms;
        if other.gpu_submissions != 0 {
            for (name, count) in &other.gpu_dispatches {
                *self.gpu_dispatches.entry(name).or_default() += count;
            }
            self.untimed_dispatches += other.untimed_dispatches;
        }
        self.upload_count += other.upload_count;
        self.upload_bytes += other.upload_bytes;
        self.readback_count += other.readback_count;
        self.readback_bytes += other.readback_bytes;
        self.readback_ms += other.readback_ms;
        self.weight_hits += other.weight_hits;
        self.weight_misses += other.weight_misses;
        self.weight_upload_bytes += other.weight_upload_bytes;
        self.weight_upload_host_ms += other.weight_upload_host_ms;
        self.pipeline_hits += other.pipeline_hits;
        self.pipeline_misses += other.pipeline_misses;
        self.pipeline_build_ms += other.pipeline_build_ms;
        for (name, time) in &other.gpu_kernel_ms {
            *self.gpu_kernel_ms.entry(name).or_default() += time;
        }
        self.peak_weight_cache_bytes = self
            .peak_weight_cache_bytes
            .max(other.peak_weight_cache_bytes);
        self.peak_batch_buffer_bytes = self
            .peak_batch_buffer_bytes
            .max(other.peak_batch_buffer_bytes);
    }
}

#[derive(Serialize)]
pub struct Diagnostics {
    pub actual: &'static str,
    pub activity: Activity,
    #[serde(flatten)]
    pub status: Status,
}

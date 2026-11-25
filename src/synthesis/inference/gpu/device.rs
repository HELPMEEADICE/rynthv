#[cfg(not(target_arch = "wasm32"))]
use super::adapter::adapters;
use super::adapter::{AdapterId, AdapterInfo, limits};
#[cfg(all(target_os = "android", feature = "gui"))]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use wgpu::{Adapter, Backends, Features, Limits};

pub(in super::super) struct Device {
    pub(super) raw: wgpu::Device,
    pub(super) queue: wgpu::Queue,
    pub(super) cache: Mutex<super::cache::Cache>,
    pub(super) limits: Limits,
    info: AdapterInfo,
    error: Arc<OnceLock<String>>,
    #[cfg(all(target_os = "android", feature = "gui"))]
    lost: Arc<AtomicBool>,
}

impl Device {
    pub(in super::super) fn fits(&self, lengths: &[usize]) -> bool {
        let limit = self
            .limits
            .max_storage_buffer_binding_size
            .min(self.limits.max_buffer_size)
            / 4;
        lengths
            .iter()
            .all(|&length| length as u64 <= limit && u32::try_from(length).is_ok())
    }

    pub(in super::super) fn info(&self) -> AdapterInfo {
        AdapterInfo {
            max_storage_binding_bytes: self.raw.limits().max_storage_buffer_binding_size,
            max_workgroups_per_dimension: self.raw.limits().max_compute_workgroups_per_dimension,
            timestamp_period_ns: self
                .raw
                .features()
                .contains(Features::TIMESTAMP_QUERY)
                .then(|| self.queue.get_timestamp_period()),
            ..self.info.clone()
        }
    }

    pub(in super::super) fn error(&self) -> Option<String> {
        self.error.get().cloned()
    }

    pub(in super::super) fn is_available(&self) -> bool {
        self.error.get().is_none()
    }

    #[cfg(all(target_os = "android", feature = "gui"))]
    pub(in super::super) fn is_lost(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    #[cfg(target_os = "android")]
    pub(in super::super) fn storage_budget(&self) -> u64 {
        self.limits
            .max_storage_buffer_binding_size
            .min(self.limits.max_buffer_size)
    }

    pub(in super::super) fn prune_weights(&self) {
        self.cache.lock().unwrap().prune_weights();
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(in super::super) fn open(
    selected: Option<&AdapterId>,
) -> (Vec<AdapterInfo>, Result<Device, String>) {
    let (backends, adapters) = adapters();
    let devices = adapters.iter().map(|(_, info)| info.clone()).collect();
    (devices, open_device(backends, adapters, selected))
}

#[cfg(not(target_arch = "wasm32"))]
fn open_device(
    backends: Backends,
    adapters: Vec<(Adapter, AdapterInfo)>,
    selected: Option<&AdapterId>,
) -> Result<Device, String> {
    if let Some(id) = selected {
        let (adapter, info) = adapters
            .into_iter()
            .find(|(_, info)| &info.id == id)
            .ok_or_else(|| format!("未找到所选 WebGPU 显卡：{id}"))?;
        if info.ambiguous {
            return Err(format!("无法区分具有相同标识的 WebGPU 显卡：{id}"));
        }
        return request_device(adapter, info);
    }
    let mut failures = Vec::new();
    for (adapter, info) in adapters {
        match request_device(adapter, info) {
            Ok(device) => return Ok(device),
            Err(error) => failures.push(error),
        }
    }
    if failures.is_empty() {
        Err(format!("No native WebGPU adapter found for {backends:?}"))
    } else {
        Err(format!(
            "No usable WebGPU inference device: {}",
            failures.join("; ")
        ))
    }
}

#[cfg(target_arch = "wasm32")]
pub(in super::super) fn open(
    selected: Option<&AdapterId>,
) -> (Vec<AdapterInfo>, Result<Device, String>) {
    let adapter = crate::platform::web::wait(async {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = Backends::BROWSER_WEBGPU;
        let instance = wgpu::Instance::new(descriptor);
        instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            })
            .await
    })
    .and_then(|result| result.map_err(|error| format!("Browser WebGPU adapter: {error}")));
    let adapter = match adapter {
        Ok(adapter) => adapter,
        Err(error) => return (Vec::new(), Err(error)),
    };
    let info = super::adapter::adapter_info(&adapter);
    let result = if selected.is_some_and(|id| *id != info.id) {
        Err("The requested adapter is unavailable in this browser".into())
    } else {
        request_device(adapter, info.clone())
    };
    (vec![info], result)
}

fn request_device(adapter: Adapter, info: AdapterInfo) -> Result<Device, String> {
    if let Some(reason) = &info.unavailable_reason {
        return Err(format!("{}: {reason}", info.id));
    }
    let descriptor = wgpu::DeviceDescriptor {
        label: Some("RynthV inference device"),
        required_features: adapter.features() & Features::TIMESTAMP_QUERY,
        required_limits: limits(adapter.limits()),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        ..Default::default()
    };
    #[cfg(not(target_arch = "wasm32"))]
    let requested = pollster::block_on(adapter.request_device(&descriptor));
    #[cfg(target_arch = "wasm32")]
    let requested =
        crate::platform::web::wait(async move { adapter.request_device(&descriptor).await })?;
    let (raw, queue) = requested.map_err(|error| format!("{}: {error}", info.id))?;
    let error = Arc::new(OnceLock::new());
    let fault = Arc::clone(&error);
    raw.on_uncaptured_error(Arc::new(move |e| {
        let _ = fault.set(format!("WebGPU inference error: {e}"));
    }));
    let fault = Arc::clone(&error);
    #[cfg(all(target_os = "android", feature = "gui"))]
    let lost = Arc::new(AtomicBool::new(false));
    #[cfg(all(target_os = "android", feature = "gui"))]
    let lost_callback = Arc::clone(&lost);
    raw.set_device_lost_callback(move |reason, message| {
        let _ = fault.set(format!(
            "WebGPU inference device lost ({reason:?}): {message}"
        ));
        #[cfg(all(target_os = "android", feature = "gui"))]
        lost_callback.store(true, Ordering::Release);
    });
    let limits = raw.limits();
    Ok(Device {
        cache: Mutex::new(super::cache::Cache::for_limits(&limits)),
        limits,
        raw,
        queue,
        info,
        error,
        #[cfg(all(target_os = "android", feature = "gui"))]
        lost,
    })
}

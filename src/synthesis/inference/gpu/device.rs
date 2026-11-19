use serde::Serialize;
use std::sync::{Arc, Mutex, OnceLock};
use wgpu::{Adapter, Backends, DeviceType, DownlevelFlags, Features, Limits};

pub(in super::super) struct Device {
    pub(super) raw: wgpu::Device,
    pub(super) queue: wgpu::Queue,
    pub(super) cache: Mutex<super::cache::Cache>,
    pub(super) limits: Limits,
    info: AdapterInfo,
    error: Arc<OnceLock<String>>,
}

#[derive(Clone, Serialize)]
pub struct AdapterInfo {
    pub name: String,
    pub api: String,
    pub device_type: String,
    pub supports_compute: bool,
    pub unavailable_reason: Option<String>,
    pub max_storage_binding_bytes: u64,
    pub max_workgroups_per_dimension: u32,
    pub timestamp_period_ns: Option<f32>,
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

    pub(in super::super) fn prune_weights(&self) {
        self.cache.lock().unwrap().prune_weights();
    }
}

pub fn list_devices() -> Vec<AdapterInfo> {
    adapters().1.iter().map(adapter_info).collect()
}

pub(in super::super) fn open() -> Result<Device, String> {
    let (backends, mut adapters) = adapters();
    adapters.sort_by_key(|adapter| {
        let info = adapter.get_info();
        let device = match info.device_type {
            DeviceType::DiscreteGpu => 0,
            DeviceType::IntegratedGpu => 1,
            DeviceType::VirtualGpu => 2,
            _ => 3,
        };
        let api = match info.backend {
            wgpu::Backend::Vulkan => 0,
            wgpu::Backend::Dx12 => 1,
            _ => 2,
        };
        (device, api)
    });
    let mut failures = Vec::new();
    for adapter in adapters {
        let info = adapter_info(&adapter);
        if let Some(reason) = &info.unavailable_reason {
            failures.push(format!("{} ({}): {reason}", info.name, info.api));
            continue;
        }
        let descriptor = wgpu::DeviceDescriptor {
            label: Some("RynthV inference device"),
            required_features: adapter.features() & Features::TIMESTAMP_QUERY,
            required_limits: limits(&adapter),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        };
        match pollster::block_on(adapter.request_device(&descriptor)) {
            Ok((raw, queue)) => {
                let error = Arc::new(OnceLock::new());
                let fault = Arc::clone(&error);
                raw.on_uncaptured_error(Arc::new(move |e| {
                    let _ = fault.set(format!("WebGPU inference error: {e}"));
                }));
                let fault = Arc::clone(&error);
                raw.set_device_lost_callback(move |reason, message| {
                    let _ = fault.set(format!(
                        "WebGPU inference device lost ({reason:?}): {message}"
                    ));
                });
                return Ok(Device {
                    limits: raw.limits(),
                    raw,
                    queue,
                    cache: Mutex::default(),
                    info,
                    error,
                });
            }
            Err(error) => failures.push(format!("{} ({}): {error}", info.name, info.api)),
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

fn adapters() -> (Backends, Vec<Adapter>) {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    let supported = if cfg!(windows) {
        Backends::DX12 | Backends::VULKAN
    } else {
        Backends::VULKAN | Backends::GL
    };
    descriptor.backends = supported & supported.with_env();
    let backends = descriptor.backends;
    if backends.is_empty() {
        return (backends, Vec::new());
    }
    let instance = wgpu::Instance::new(descriptor);
    (
        backends,
        pollster::block_on(instance.enumerate_adapters(backends)),
    )
}

fn limits(adapter: &Adapter) -> Limits {
    let supported = adapter.limits();
    Limits {
        max_storage_buffers_per_shader_stage: 5,
        max_buffer_size: supported.max_buffer_size,
        max_storage_buffer_binding_size: supported.max_storage_buffer_binding_size,
        max_compute_workgroups_per_dimension: supported.max_compute_workgroups_per_dimension,
        ..Limits::downlevel_defaults()
    }
    .using_alignment(supported)
}

fn adapter_info(adapter: &Adapter) -> AdapterInfo {
    let info = adapter.get_info();
    let supports_compute = adapter
        .get_downlevel_capabilities()
        .flags
        .contains(DownlevelFlags::COMPUTE_SHADERS);
    let unavailable_reason = if info.device_type == DeviceType::Cpu {
        Some("software adapter".into())
    } else if !supports_compute {
        Some("compute shaders unavailable".into())
    } else if !limits(adapter).check_limits(&adapter.limits()) {
        Some("insufficient WebGPU limits for inference".into())
    } else {
        None
    };
    AdapterInfo {
        name: info.name,
        api: match info.backend {
            wgpu::Backend::Dx12 => "DX12".into(),
            backend => format!("{backend:?}"),
        },
        device_type: format!("{:?}", info.device_type),
        supports_compute,
        unavailable_reason,
        max_storage_binding_bytes: adapter.limits().max_storage_buffer_binding_size,
        max_workgroups_per_dimension: adapter.limits().max_compute_workgroups_per_dimension,
        timestamp_period_ns: None,
    }
}

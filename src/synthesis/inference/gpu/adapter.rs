use serde::{Deserialize, Serialize};
use std::fmt;
#[cfg(not(target_arch = "wasm32"))]
use wgpu::Backends;
use wgpu::{Adapter, DeviceType, DownlevelFlags, Limits};

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterId {
    name: String,
    api: String,
    vendor_id: u32,
    device_id: u32,
    pci_bus_id: String,
}

impl fmt::Display for AdapterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} · {}", self.name, self.api)?;
        if !self.pci_bus_id.is_empty() {
            write!(f, " · {}", self.pci_bus_id)?;
        }
        Ok(())
    }
}

#[derive(Clone, Serialize)]
pub struct AdapterInfo {
    #[serde(flatten)]
    pub id: AdapterId,
    pub device_type: String,
    pub driver: String,
    pub driver_info: String,
    pub supports_compute: bool,
    pub unavailable_reason: Option<String>,
    pub max_storage_binding_bytes: u64,
    pub max_workgroups_per_dimension: u32,
    pub timestamp_period_ns: Option<f32>,
    #[serde(skip)]
    pub(super) ambiguous: bool,
}

impl AdapterInfo {
    pub fn is_selectable(&self) -> bool {
        self.unavailable_reason.is_none() && !self.ambiguous
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn list_devices() -> Vec<AdapterInfo> {
    adapters().1.into_iter().map(|(_, info)| info).collect()
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn adapters() -> (Backends, Vec<(Adapter, AdapterInfo)>) {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    let supported = if cfg!(windows) {
        Backends::DX12 | Backends::VULKAN
    } else if cfg!(all(target_os = "linux", feature = "gui"))
        && (std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some())
    {
        Backends::VULKAN | Backends::GL
    } else {
        Backends::VULKAN
    };
    descriptor.backends = supported & supported.with_env();
    let backends = descriptor.backends;
    if backends.is_empty() {
        return (backends, Vec::new());
    }
    let instance = wgpu::Instance::new(descriptor);
    let mut adapters = pollster::block_on(instance.enumerate_adapters(backends));
    adapters.sort_by_key(adapter_priority);
    let mut entries: Vec<_> = adapters
        .into_iter()
        .map(|adapter| {
            let info = adapter_info(&adapter);
            (adapter, info)
        })
        .collect();
    // Some drivers give identical IDs to multiple cards. Avoid persisting a
    // choice that would depend on their enumeration order.
    for index in 0..entries.len() {
        let id = &entries[index].1.id;
        entries[index].1.ambiguous = entries.iter().filter(|(_, info)| &info.id == id).count() > 1;
    }
    (backends, entries)
}

#[cfg(not(target_arch = "wasm32"))]
fn adapter_priority(adapter: &Adapter) -> (u8, u8) {
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
}

pub(super) fn limits(supported: Limits) -> Limits {
    Limits {
        max_storage_buffers_per_shader_stage: 5,
        max_buffer_size: supported.max_buffer_size,
        max_storage_buffer_binding_size: supported.max_storage_buffer_binding_size,
        max_compute_workgroups_per_dimension: supported.max_compute_workgroups_per_dimension,
        ..Limits::downlevel_defaults()
    }
    .using_alignment(supported)
}

pub(super) fn adapter_info(adapter: &Adapter) -> AdapterInfo {
    let info = adapter.get_info();
    let supported = adapter.limits();
    let supports_compute = adapter
        .get_downlevel_capabilities()
        .flags
        .contains(DownlevelFlags::COMPUTE_SHADERS);
    let unavailable_reason = if info.device_type == DeviceType::Cpu {
        Some("software adapter".into())
    } else if !supports_compute {
        Some("compute shaders unavailable".into())
    } else if !limits(supported.clone()).check_limits(&supported) {
        Some("insufficient WebGPU limits for inference".into())
    } else {
        None
    };
    AdapterInfo {
        id: AdapterId {
            name: info.name,
            api: match info.backend {
                wgpu::Backend::Dx12 => "DX12".into(),
                backend => format!("{backend:?}"),
            },
            vendor_id: info.vendor,
            device_id: info.device,
            pci_bus_id: info.device_pci_bus_id,
        },
        device_type: format!("{:?}", info.device_type),
        driver: info.driver,
        driver_info: info.driver_info,
        supports_compute,
        unavailable_reason,
        max_storage_binding_bytes: supported.max_storage_buffer_binding_size,
        max_workgroups_per_dimension: supported.max_compute_workgroups_per_dimension,
        timestamp_period_ns: None,
        ambiguous: false,
    }
}

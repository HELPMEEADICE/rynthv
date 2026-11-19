//! Native surface-free WebGPU inference.

mod device;

pub use device::{AdapterInfo, list_devices};
pub(super) use device::{Device, open};

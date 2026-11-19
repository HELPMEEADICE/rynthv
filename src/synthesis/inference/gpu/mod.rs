//! Native surface-free WebGPU inference.

mod batch;
mod cache;
mod convolution;
mod device;
mod gru;
mod linear;

pub(super) use batch::Batch;
pub(super) use convolution::{convolution, transpose};
pub use device::{AdapterInfo, list_devices};
pub(super) use device::{Device, open};
pub(super) use gru::{bidirectional_gru, gru};
pub(super) use linear::affine;

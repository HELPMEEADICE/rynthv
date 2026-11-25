//! Surface-free WebGPU inference, independent of the editor renderer.

mod adapter;
mod attention;
mod batch;
mod cache;
mod chain;
mod conditional;
mod convolution;
mod device;
mod elementwise;
mod gru;
mod linear;
mod timestamps;
mod validate;

#[cfg(not(target_arch = "wasm32"))]
pub use adapter::list_devices;
pub use adapter::{AdapterId, AdapterInfo};
pub(super) use attention::{attention, projected_attention};
pub(super) use batch::Batch;
pub(super) use chain::sequence;
pub(super) use conditional::{ConditionalGraph, ConditionalProjection};
pub(super) use convolution::{convolution, transpose};
pub(super) use device::{Device, open};
pub(super) use elementwise::{Element, elementwise};
pub(super) use gru::{bidirectional_gru, gru};
pub(super) use linear::affine;
pub(super) use validate::validate;

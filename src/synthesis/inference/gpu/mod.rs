//! Native surface-free WebGPU inference.

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

pub(super) use attention::{attention, projected_attention};
pub(super) use batch::Batch;
pub(super) use chain::sequence;
pub(super) use conditional::ConditionalGraph;
pub(super) use convolution::{convolution, transpose};
pub use device::{AdapterInfo, list_devices};
pub(super) use device::{Device, open};
pub(super) use elementwise::{Element, elementwise};
pub(super) use gru::{bidirectional_gru, gru};
pub(super) use linear::affine;
pub(super) use validate::validate;

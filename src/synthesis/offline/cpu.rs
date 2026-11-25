//! Web CPU owner inference over a bounded pool of independent WASM instances.
mod export;
mod pool;
mod protocol;
mod worker;

pub(super) use pool::{JobEvent, Pool};
pub(super) use protocol::Update;
pub(crate) use protocol::{Activity, Job};
pub(crate) use worker::run_job;

use super::*;

impl Offline {
    pub(super) fn use_cpu_workers(&self, backend: Backend) -> bool {
        match backend {
            Backend::Cpu => true,
            Backend::Auto => {
                let status = self
                    .compute
                    .as_ref()
                    .expect("prepared compute session")
                    .2
                    .status();
                status.device.is_none() || status.error.is_some()
            }
            Backend::WebGpu => false,
        }
    }
}

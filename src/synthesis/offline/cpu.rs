//! Web CPU owner inference over a bounded pool of independent WASM instances.
mod export;
mod pool;
mod protocol;
mod worker;

pub(super) use pool::{JobEvent, Pool};
pub(super) use protocol::Update;
pub(crate) use protocol::{Activity, Job};
pub(crate) use worker::run_job;

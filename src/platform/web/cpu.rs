#![allow(
    deprecated,
    reason = "wasm-bindgen currently marks JSPI support experimental"
)]

use super::{js_error, worker};
use crate::synthesis::{
    execution::{self, Cancellation},
    offline::cpu,
};
use js_sys::{Array, Float32Array, Promise};
use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(raw_module = "./cpu-pool.js")]
extern "C" {
    pub(crate) fn cpu_slots() -> usize;
    pub(crate) fn start_cpu_job(slot: usize, job: &str);
    pub(crate) fn prioritize_cpu_job(slot: usize, seconds: Option<f64>);
    pub(crate) fn cancel_cpu_job(slot: usize);
    pub(crate) fn poll_cpu_packet() -> Option<Array>;
    fn wait_cpu_packets() -> Promise;
    fn stop_cpu_jobs() -> Promise;
}

#[wasm_bindgen(raw_module = "./cpu-worker-bridge.js")]
extern "C" {
    fn cpu_progress(message: &str, pcm: Option<Float32Array>) -> Promise;
}

pub(crate) fn wait_packets() -> Result<(), String> {
    wait(wait_cpu_packets())
}

pub(crate) fn stop_jobs() -> Result<(), String> {
    wait(stop_cpu_jobs())
}

pub(crate) fn progress(
    event: &impl serde::Serialize,
    samples: Option<&[f32]>,
) -> Result<(), String> {
    let message = serde_json::to_string(event).map_err(|error| error.to_string())?;
    wait(cpu_progress(&message, samples.map(Float32Array::from)))
}

fn wait(promise: Promise) -> Result<(), String> {
    wasm_bindgen_futures::jspi_block_on_promise(&promise)
        .map(drop)
        .map_err(js_error)
}

thread_local! {
    static CANCEL: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
    static DEMAND: Cell<Option<f64>> = const { Cell::new(None) };
}

#[wasm_bindgen(js_name = cancel_cpu_job)]
pub fn request_cancel() {
    CANCEL.with_borrow(|cancel| {
        if let Some(cancel) = cancel {
            cancel.store(true, Ordering::Release);
        }
    });
}

#[wasm_bindgen(js_name = prioritize_cpu_job)]
pub fn request_priority(seconds: Option<f64>) {
    DEMAND.set(seconds);
}

#[wasm_bindgen(jspi)]
pub fn run_cpu_job(job: &str, new_demand: bool, seconds: Option<f64>) -> Result<Array, JsError> {
    let mut job: cpu::Job = serde_json::from_str(job)?;
    if new_demand {
        job.demand = seconds;
    }
    DEMAND.set(job.demand);
    let cancel = Arc::new(AtomicBool::new(false));
    CANCEL.set(Some(Arc::clone(&cancel)));
    let result = worker::with_event_pump(|| {
        execution::scope(Cancellation::from_flag(Arc::clone(&cancel)), || {
            cpu::run_job(job, &cancel, || DEMAND.get())
        })
    });
    CANCEL.set(None);
    result.map_err(|error| JsError::new(&error))
}

//! One active request owns the worker's inference session and OPFS handles.
#![allow(
    deprecated,
    reason = "wasm-bindgen currently marks JSPI support experimental"
)]

mod render;
mod resources;

use super::{
    fs, js_error,
    protocol::{Event, Request},
};
use crate::synthesis::{offline::Offline, settings::SynthesisSettings};
use js_sys::{Float32Array, Promise};
use serde::Serialize;
use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use wasm_bindgen::prelude::*;
use web_time::Instant;

#[wasm_bindgen(raw_module = "./worker-bridge.js")]
extern "C" {
    fn emit_event(message: &str, samples: Option<Float32Array>);
    fn yield_worker() -> Promise;
}

#[derive(Default)]
struct Worker {
    offline: Offline,
    settings: Option<SynthesisSettings>,
}

thread_local! {
    static WORKER: RefCell<Worker> = RefCell::default();
    static CANCELLATION: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
    static DEMAND: Cell<f64> = const { Cell::new(0.0) };
    static LAST_YIELD: Cell<Option<Instant>> = const { Cell::new(None) };
}

#[wasm_bindgen]
pub fn cancel_worker_request() {
    CANCELLATION.with_borrow(|value| {
        if let Some(flag) = value {
            flag.store(true, Ordering::Release);
        }
    });
}

#[wasm_bindgen]
pub fn prioritize_worker_preview(seconds: f64) {
    if seconds.is_finite() {
        DEMAND.set(seconds.max(0.0));
    }
}

fn preview_demand() -> f64 {
    DEMAND.get()
}

#[wasm_bindgen(jspi)]
pub fn run_worker_request(request: &str, position: Option<f64>) -> Result<String, JsError> {
    let mut request: Request = serde_json::from_str(request)?;
    if let (
        Request::Preview {
            position: initial, ..
        },
        Some(position),
    ) = (&mut request, position)
    {
        *initial = position;
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    CANCELLATION.with(|value| value.replace(Some(Arc::clone(&cancelled))));
    let result =
        with_event_pump(|| WORKER.with_borrow_mut(|worker| worker.run(request, cancelled)));
    CANCELLATION.with(|value| value.take());
    result.map_err(|error| JsError::new(&error))
}

impl Worker {
    fn run(&mut self, request: Request, cancelled: Arc<AtomicBool>) -> Result<String, String> {
        if self.settings.is_none() {
            self.settings = Some(SynthesisSettings::load()?);
        }
        match request {
            Request::Scan => self.library(None),
            Request::ImportVoices { paths } => self.import_voices(paths),
            Request::ImportDictionaries { path } => self.import_dictionaries(&path),
            Request::Settings { settings } => self.update_settings(settings),
            Request::RemoveVoice { selection } => self.remove_voice(&selection),
            Request::Inventory { selection } => self.inventory(&selection),
            Request::Context {
                name,
                version,
                file,
                identity,
            } => self.import_context(&name, version, file.as_deref(), identity),
            Request::Midi { path } => {
                let result = crate::midi::load(&path);
                self.remove_staged(&path);
                json(&result?)
            }
            Request::Audio { path } => {
                let asset = self.offline.recording(&path, &cancelled)?;
                let mut peaks = Vec::new();
                for wave in &asset.waveforms {
                    wave.append_peaks(&mut peaks);
                }
                let audio = &asset.audio;
                event(
                    Event::Audio {
                        channels: audio.layout().channels(),
                        rate: audio.sample_rate(),
                        frames: audio.frames(),
                    },
                    Some(&peaks),
                );
                json(&())
            }
            Request::Preview { project, position } => {
                prioritize_worker_preview(position);
                self.render(&project, Some(preview_demand), cancelled)
            }
            Request::Export { project } => self.render(&project, None, cancelled),
        }
    }

    fn settings(&self) -> &SynthesisSettings {
        self.settings
            .as_ref()
            .expect("initialized browser synthesis settings")
    }
}

fn json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}

fn event(value: Event, samples: Option<&[f32]>) {
    let message = json(&value).expect("serializable worker event");
    let samples = samples.map(Float32Array::from);
    emit_event(&message, samples);
}

fn yield_events() -> Result<(), String> {
    #[allow(deprecated)]
    wasm_bindgen_futures::jspi_block_on_promise(&yield_worker())
        .map(drop)
        .map_err(js_error)
}

pub(super) fn with_event_pump<T>(run: impl FnOnce() -> T) -> T {
    LAST_YIELD.set(Some(Instant::now()));
    crate::synthesis::execution::with_event_pump(pump_events, run)
}

fn pump_events() {
    if LAST_YIELD
        .get()
        .is_some_and(|last| last.elapsed() >= Duration::from_millis(16))
    {
        let _ = yield_events();
        LAST_YIELD.set(Some(Instant::now()));
    }
}

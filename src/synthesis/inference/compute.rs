//! Compute selection belongs to a render scope, independent of the editor renderer.

use web_time::Instant;
mod diagnostics;

use super::{
    gpu::{Batch, Device, open},
    matrix::MatrixError,
};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg(not(target_arch = "wasm32"))]
pub use super::gpu::list_devices;
pub use super::gpu::{AdapterId, AdapterInfo};
pub use diagnostics::{Activity, Diagnostics};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    #[default]
    Auto,
    Cpu,
    WebGpu,
}

#[derive(Clone)]
pub struct Session(Arc<Inner>);

struct Inner {
    requested: Backend,
    adapters: Vec<AdapterInfo>,
    device: Option<Arc<Device>>,
    initialization_error: Option<String>,
    initialization_ms: f64,
    activity: Mutex<Activity>,
    timestamps: AtomicBool,
}

#[derive(Serialize, Deserialize)]
pub struct Status {
    pub requested: Backend,
    pub device: Option<AdapterInfo>,
    pub error: Option<String>,
    pub initialization_ms: f64,
}

thread_local! {
    static CURRENT: RefCell<Option<Session>> = const { RefCell::new(None) };
}

impl Session {
    /// May initialize a native device. Call from a synthesis or setup thread.
    pub fn new(requested: Backend, adapter: Option<&AdapterId>) -> Self {
        let started = Instant::now();
        let (adapters, device, initialization_error) = match requested {
            Backend::Cpu => (Vec::new(), None, None),
            Backend::Auto | Backend::WebGpu => {
                let (adapters, result) = open(adapter);
                match result {
                    Ok(device) => (adapters, Some(Arc::new(device)), None),
                    Err(error) => (adapters, None, Some(error)),
                }
            }
        };
        Self(Arc::new(Inner {
            requested,
            adapters,
            device,
            initialization_error,
            initialization_ms: started.elapsed().as_secs_f64() * 1000.0,
            activity: Mutex::default(),
            timestamps: AtomicBool::new(false),
        }))
    }

    pub(crate) fn unavailable(requested: Backend, reason: String) -> Self {
        Self(Arc::new(Inner {
            requested,
            adapters: Vec::new(),
            device: None,
            initialization_error: Some(reason),
            initialization_ms: 0.0,
            activity: Mutex::default(),
            timestamps: AtomicBool::new(false),
        }))
    }

    pub fn adapters(&self) -> &[AdapterInfo] {
        &self.0.adapters
    }

    pub fn status(&self) -> Status {
        Status {
            requested: self.0.requested,
            device: self.0.device.as_deref().map(Device::info),
            error: self
                .0
                .device
                .as_deref()
                .and_then(Device::error)
                .or_else(|| self.0.initialization_error.clone()),
            initialization_ms: self.0.initialization_ms,
        }
    }

    pub fn check(&self) -> Result<(), String> {
        if self.0.requested != Backend::WebGpu {
            return Ok(());
        }
        match &self.0.device {
            Some(device) => device.error().map_or(Ok(()), Err),
            None => Err(self
                .0
                .initialization_error
                .clone()
                .unwrap_or_else(|| "No WebGPU inference device".into())),
        }
    }

    pub fn scope<T>(&self, run: impl FnOnce() -> T) -> T {
        scope(Some(self.clone()), run)
    }

    pub(crate) fn prune_weights(&self) {
        if let Some(device) = &self.0.device {
            device.prune_weights();
        }
    }

    #[cfg(all(target_os = "android", feature = "gui"))]
    pub(crate) fn device_lost(&self) -> bool {
        self.0.device.as_deref().is_some_and(Device::is_lost)
    }

    #[cfg(feature = "synth-profile")]
    pub fn enable_timestamps(&self) {
        self.0.timestamps.store(true, Ordering::Relaxed);
    }

    pub fn diagnostics(&self) -> Diagnostics {
        self.describe(self.0.activity.lock().unwrap().clone())
    }

    /// Capture one measurement while retaining device resources for the next render.
    pub fn take_diagnostics(&self) -> Diagnostics {
        self.describe(std::mem::take(&mut *self.0.activity.lock().unwrap()))
    }

    fn describe(&self, activity: Activity) -> Diagnostics {
        let actual = match (
            activity.completed_gpu_submissions,
            activity.cpu_operators.is_empty(),
        ) {
            (0, true) => "idle",
            (0, false) => "cpu",
            (_, true) => "webgpu",
            (_, false) => "mixed",
        };
        Diagnostics {
            actual,
            activity,
            status: self.status(),
        }
    }

    /// Compile every WGSL entry point with the selected device's backend.
    pub fn validate_shaders(&self) -> Result<Vec<&'static str>, String> {
        self.check()?;
        let device = self.0.device.as_ref().ok_or("No WebGPU inference device")?;
        super::gpu::validate(device)
    }

    fn meets_gpu_threshold(&self, work: usize, minimum: usize) -> bool {
        work >= minimum
            || match self.0.requested {
                Backend::Auto => false,
                Backend::WebGpu if cfg!(target_arch = "wasm32") => {
                    self.0.activity.lock().unwrap().completed_gpu_submissions == 0
                }
                _ => true,
            }
    }

    fn record_cpu(&self, operator: &'static str, reason: &'static str) {
        *self
            .0
            .activity
            .lock()
            .unwrap()
            .cpu_operators
            .entry(operator)
            .or_default()
            .entry(reason)
            .or_default() += 1;
    }
}

pub(crate) fn record_cpu(operator: &'static str, reason: &'static str) {
    if let Some(session) = current() {
        session.record_cpu(operator, reason);
    }
}

pub(super) fn accelerate(
    operator: impl Into<Option<&'static str>>,
    work: usize,
    min_auto_work: usize,
    lengths: &[usize],
    run: impl FnOnce(&mut Batch<'_>) -> Result<(), MatrixError>,
) -> Result<bool, MatrixError> {
    let operator = operator.into();
    let Some(session) = current() else {
        return Ok(false);
    };
    let Some(device) = &session.0.device else {
        session.check().map_err(MatrixError::Compute)?;
        if let Some(operator) = operator {
            session.record_cpu(
                operator,
                if session.0.requested == Backend::Cpu {
                    "CPU selected"
                } else {
                    "GPU unavailable"
                },
            );
        }
        return Ok(false);
    };
    if !device.is_available() && session.0.requested == Backend::Auto {
        if let Some(operator) = operator {
            session.record_cpu(operator, "GPU execution unavailable");
        }
        return Ok(false);
    }
    if !session.meets_gpu_threshold(work, min_auto_work) {
        if let Some(operator) = operator {
            session.record_cpu(operator, "below GPU scheduling threshold");
        }
        return Ok(false);
    }
    if !device.fits(lengths) {
        if let Some(operator) = operator {
            session.record_cpu(operator, "tensor exceeds WebGPU storage limits");
        }
        return Ok(false);
    }
    let mut batch = Batch::new(device, session.0.timestamps.load(Ordering::Relaxed))?;
    let result = run(&mut batch);
    session.0.activity.lock().unwrap().merge(batch.activity());
    result.map(|()| true)
}

pub(super) fn record_readback(device: &Arc<Device>, batch: &Batch<'_>) {
    if let Some(session) = current()
        && session
            .0
            .device
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, device))
    {
        session.0.activity.lock().unwrap().merge(batch.activity());
    }
}

/// A stage appended to a GPU batch shares its submission and readback.
pub(super) fn can_append_gpu(lengths: &[usize]) -> bool {
    current().is_some_and(|session| {
        session
            .0
            .device
            .as_ref()
            .is_some_and(|device| device.is_available() && device.fits(lengths))
    })
}

pub fn current() -> Option<Session> {
    CURRENT.with(|current| current.borrow().clone())
}

pub(super) fn has_gpu() -> bool {
    CURRENT.with(|current| {
        current
            .borrow()
            .as_ref()
            .and_then(|session| session.0.device.as_deref())
            .is_some_and(Device::is_available)
    })
}

#[cfg(target_os = "android")]
pub(super) fn gpu_storage_budget() -> Option<u64> {
    CURRENT.with_borrow(|current| {
        let device = current.as_ref()?.0.device.as_ref()?;
        Some(device.storage_budget())
    })
}

pub fn scope<T>(session: Option<Session>, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<Session>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT.with(|current| current.replace(self.0.take()));
        }
    }
    let _restore = Restore(CURRENT.with(|current| current.replace(session)));
    run()
}

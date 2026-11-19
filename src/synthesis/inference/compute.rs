//! Compute selection belongs to a render scope, independent of the editor renderer.

mod diagnostics;

use super::{
    gpu::{Batch, Device, open},
    matrix::MatrixError,
};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    sync::{Arc, Mutex},
    time::Instant,
};

pub use super::gpu::{AdapterInfo, list_devices};
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
    device: Option<Device>,
    initialization_error: Option<String>,
    initialization_ms: f64,
    activity: Mutex<Activity>,
}

#[derive(Serialize)]
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
    pub fn new(requested: Backend) -> Self {
        let started = Instant::now();
        let (device, initialization_error) = match requested {
            Backend::Cpu => (None, None),
            Backend::Auto | Backend::WebGpu => match open() {
                Ok(device) => (Some(device), None),
                Err(error) => (None, Some(error)),
            },
        };
        Self(Arc::new(Inner {
            requested,
            device,
            initialization_error,
            initialization_ms: started.elapsed().as_secs_f64() * 1000.0,
            activity: Mutex::default(),
        }))
    }

    pub fn status(&self) -> Status {
        Status {
            requested: self.0.requested,
            device: self.0.device.as_ref().map(Device::info),
            error: self
                .0
                .device
                .as_ref()
                .and_then(Device::error)
                .or_else(|| self.0.initialization_error.clone()),
            initialization_ms: self.0.initialization_ms,
        }
    }

    pub fn check(&self) -> Result<(), String> {
        match &self.0.device {
            Some(device) => device.error().map_or(Ok(()), Err),
            None if self.0.requested == Backend::WebGpu => Err(self
                .0
                .initialization_error
                .clone()
                .unwrap_or_else(|| "No WebGPU inference device".into())),
            None => Ok(()),
        }
    }

    pub fn scope<T>(&self, run: impl FnOnce() -> T) -> T {
        scope(Some(self.clone()), run)
    }

    pub fn diagnostics(&self) -> Diagnostics {
        let activity = self.0.activity.lock().unwrap().clone();
        let actual = match (activity.gpu_submissions, activity.cpu_operators.is_empty()) {
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

pub(super) fn record_cpu(operator: &'static str, reason: &'static str) {
    if let Some(session) = current() {
        session.record_cpu(operator, reason);
    }
}

pub(super) fn accelerate(
    operator: &'static str,
    work: usize,
    min_auto_work: usize,
    lengths: &[usize],
    run: impl FnOnce(&mut Batch<'_>) -> Result<(), MatrixError>,
) -> Result<bool, MatrixError> {
    let Some(session) = current() else {
        return Ok(false);
    };
    let Some(device) = &session.0.device else {
        session.check().map_err(MatrixError::Compute)?;
        session.record_cpu(
            operator,
            if session.0.requested == Backend::Cpu {
                "CPU selected"
            } else {
                "GPU unavailable"
            },
        );
        return Ok(false);
    };
    if session.0.requested == Backend::Auto && work < min_auto_work {
        session.record_cpu(operator, "below GPU scheduling threshold");
        return Ok(false);
    }
    if !device.fits(lengths) {
        session.record_cpu(operator, "tensor exceeds WebGPU storage limits");
        return Ok(false);
    }
    let mut batch = Batch::new(device)?;
    let result = run(&mut batch);
    session.0.activity.lock().unwrap().merge(batch.activity());
    result.map(|()| true)
}

pub fn current() -> Option<Session> {
    CURRENT.with(|current| current.borrow().clone())
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

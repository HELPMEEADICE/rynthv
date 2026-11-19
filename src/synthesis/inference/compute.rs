//! Compute selection belongs to a render scope, independent of the editor renderer.

use super::gpu::{Device, open};
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, sync::Arc, time::Instant};

pub use super::gpu::{AdapterInfo, list_devices};

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

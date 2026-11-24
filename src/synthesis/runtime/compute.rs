use super::{Runtime, events::WakeOnExit};
use crate::synthesis::inference::compute::{AdapterId, AdapterInfo, Backend, Diagnostics, Session};
use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

pub(super) enum State {
    Loading(Backend, Receiver<Session>),
    Ready(Session),
}

impl State {
    pub(super) fn new(
        backend: Backend,
        adapter: Option<AdapterId>,
        repaint: crate::synthesis::notify::Notify,
    ) -> Self {
        if backend == Backend::Cpu {
            return Self::Ready(Session::new(backend, None));
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        match thread::Builder::new()
            .name("inference-device".into())
            .spawn(move || {
                let _wake = WakeOnExit(repaint);
                let _ = sender.send(Session::new(backend, adapter.as_ref()));
            }) {
            Ok(_) => Self::Loading(backend, receiver),
            Err(error) => Self::Ready(Session::unavailable(
                backend,
                format!("无法启动推理设备初始化线程：{error}"),
            )),
        }
    }

    pub(super) fn session(&self) -> Option<&Session> {
        match self {
            Self::Loading(..) => None,
            Self::Ready(session) => Some(session),
        }
    }

    pub(super) fn poll(&mut self) -> bool {
        let Self::Loading(backend, receiver) = self else {
            return false;
        };
        *self = match receiver.try_recv() {
            Ok(session) => Self::Ready(session),
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Self::Ready(Session::unavailable(
                *backend,
                "推理设备初始化线程意外退出。".into(),
            )),
        };
        true
    }
}

impl Runtime {
    pub fn compute_diagnostics(&self) -> Option<Diagnostics> {
        self.compute.session().map(Session::diagnostics)
    }

    pub fn compute_adapters(&self) -> Option<&[AdapterInfo]> {
        self.compute.session().map(Session::adapters)
    }

    pub fn set_compute_settings(&mut self, backend: Backend, adapter: Option<AdapterId>) {
        let current = &self.library.settings;
        if current.compute_backend == backend && current.compute_adapter == adapter {
            return;
        }
        let mut settings = current.clone();
        settings.compute_backend = backend;
        settings.compute_adapter = adapter.clone();
        if self.library.update(settings) {
            // Closing worker channels cancels active jobs and prevents publication
            // from the previous device. New workers start with fresh render caches.
            self.clear_voices();
            self.visuals.clear();
            self.compute = State::new(backend, adapter, self.repaint.clone());
        }
    }
}

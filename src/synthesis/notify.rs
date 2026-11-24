use std::sync::Arc;

/// Wakes a consumer when background synthesis or audio work publishes a result.
#[derive(Clone)]
pub struct Notify(Arc<dyn Fn() + Send + Sync>);

impl Notify {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(wake))
    }

    pub(crate) fn wake(&self) {
        (self.0)();
    }
}

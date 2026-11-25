use crate::platform::android::{lifecycle, memory_bytes};
use std::{
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

static ACTIVE: Mutex<usize> = Mutex::new(0);
static CHANGED: Condvar = Condvar::new();

pub(super) struct Permit;

impl Permit {
    pub(super) fn acquire(alive: &AtomicBool) -> Option<Self> {
        let limit = if memory_bytes() >= 8 * (1 << 30) {
            2
        } else {
            1
        };
        let mut active = ACTIVE.lock().ok()?;
        while alive.load(Ordering::Acquire) {
            if lifecycle::foreground() && *active < limit {
                *active += 1;
                return Some(Self);
            }
            active = CHANGED
                .wait_timeout(active, Duration::from_millis(200))
                .ok()?
                .0;
        }
        None
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE.lock() {
            *active -= 1;
            CHANGED.notify_one();
        }
    }
}

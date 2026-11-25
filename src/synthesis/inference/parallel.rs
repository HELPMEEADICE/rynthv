//! The bounded, persistent operator pool shared by independent-frame kernels.

use rayon::{ThreadPool, ThreadPoolBuilder};
use std::sync::OnceLock;

static POOL: OnceLock<Result<ThreadPool, String>> = OnceLock::new();

pub(super) fn pool() -> Result<&'static ThreadPool, String> {
    POOL.get_or_init(|| {
        let available = std::thread::available_parallelism()
            .map_err(|error| format!("Cannot determine synthesis concurrency: {error}"))?
            .get();
        let threads = match std::env::var("RYNTHV_THREADS") {
            Ok(value) => value
                .parse::<usize>()
                .ok()
                .filter(|threads| (1..=256).contains(threads))
                .ok_or("RYNTHV_THREADS must be an integer between 1 and 256")?,
            Err(std::env::VarError::NotPresent) => default_threads(available),
            Err(error) => return Err(error.to_string()),
        };
        ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|index| format!("synthesis-compute-{index}"))
            .build()
            .map_err(|error| format!("Cannot prepare synthesis compute pool: {error}"))
    })
    .as_ref()
    .map_err(Clone::clone)
}

fn default_threads(available: usize) -> usize {
    #[cfg(all(target_os = "android", feature = "gui"))]
    {
        let low_memory = crate::platform::android::memory_bytes() < 4 * (1 << 30);
        available
            .saturating_sub(1)
            .clamp(1, if low_memory { 2 } else { 4 })
    }
    #[cfg(not(all(target_os = "android", feature = "gui")))]
    {
        available.min(8)
    }
}

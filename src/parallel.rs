//! Explicit application pool setup and a non-initializing worker-count policy.
use crate::Result;
use std::sync::atomic::{AtomicUsize, Ordering};

static GLOBAL_WORKERS: AtomicUsize = AtomicUsize::new(0);

/// Configure the process-wide Rayon pool once, using 1..=64 workers.
///
/// The CLI and optional desktop call this at startup. Embedders may instead use
/// their own Rayon pool and `install`. A failed or repeated setup leaves the
/// previously registered policy unchanged. Querying this policy does not create
/// a pool. Operations that use it stay serial when no caller or registered
/// global pool exists.
pub fn initialize_worker_pool(workers: usize) -> Result<()> {
    if !(1..=64).contains(&workers) {
        return Err("Worker count must be between 1 and 64".into());
    }
    // wasm32 has no threads: the one pool worker is the calling thread itself,
    // so Rayon never tries to spawn and every parallel kernel runs serially.
    #[cfg(target_arch = "wasm32")]
    let (workers, builder) = (1, rayon::ThreadPoolBuilder::new().use_current_thread());
    #[cfg(not(target_arch = "wasm32"))]
    let builder = rayon::ThreadPoolBuilder::new();
    builder
        .num_threads(workers)
        .build_global()
        .map_err(|error| error.to_string())?;
    GLOBAL_WORKERS.store(workers, Ordering::Release);
    Ok(())
}

/// Zero means that this caller has neither a local nor a registered global pool.
/// Check TLS before asking Rayon for its count: its off-pool count query would
/// otherwise lazily initialize an unconfigured global pool.
pub(crate) fn current_worker_count() -> usize {
    if rayon::current_thread_index().is_some() {
        rayon::current_num_threads()
    } else {
        GLOBAL_WORKERS.load(Ordering::Acquire)
    }
}

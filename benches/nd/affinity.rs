//! Benchmark-only affinity: each worker uses one specified physical core.
//! The caller supplies CPUs from its allowed affinity set; no other process changes.
pub fn bind_worker(index: usize) {
    #[cfg(target_os = "linux")]
    {
        let Ok(cpus) = std::env::var("ND_BENCH_CPUS") else {
            return;
        };
        let cpus: Vec<usize> = cpus.split(',').map(|v| v.parse().unwrap()).collect();
        let cpu = cpus[index % cpus.len()];
        let mut mask = [0usize; 16];
        let bits = usize::BITS as usize;
        assert!(cpu < mask.len() * bits);
        mask[cpu / bits] |= 1usize << (cpu % bits);
        unsafe extern "C" {
            fn sched_setaffinity(pid: i32, size: usize, mask: *const usize) -> i32;
        }
        // SAFETY: pid 0 selects only this worker, and mask is valid for size bytes.
        let status = unsafe { sched_setaffinity(0, std::mem::size_of_val(&mask), mask.as_ptr()) };
        assert_eq!(
            status, 0,
            "cannot bind benchmark worker {index} to CPU {cpu}"
        );
    }
    #[cfg(not(target_os = "linux"))]
    let _ = index;
}

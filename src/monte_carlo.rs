use std::panic;
use std::sync::Arc;
use std::thread;

/// Run `f` once per seed on a dedicated OS thread and return the results in
/// seed order.
///
/// Each invocation of `f` receives a seed and is responsible for constructing
/// its own [`SimEnv`](crate::env::SimEnv) via
/// [`SimEnv::with_seed`](crate::env::SimEnv::with_seed). Because `SimEnv` is
/// created *inside* the closure it never crosses thread boundaries, so its
/// `!Send` nature is not a problem.
///
/// `F` is wrapped in an `Arc` and shared across threads, so it must be
/// `Send + Sync`. A plain function pointer or a closure that captures only
/// `Send + Sync` data satisfies this automatically.
///
/// # Panics
///
/// If any worker thread panics, the original panic payload is re-raised on
/// the calling thread via [`std::panic::resume_unwind`], preserving the
/// original backtrace. Surviving threads are still joined before the
/// re-raise, so no threads are orphaned.
pub fn run<F, R>(seeds: impl IntoIterator<Item = u64>, f: F) -> Vec<R>
where
    F: Fn(u64) -> R + Send + Sync + 'static,
    R: Send + 'static,
{
    let f = Arc::new(f);

    let handles: Vec<_> = seeds
        .into_iter()
        .map(|seed| {
            let f = Arc::clone(&f);
            thread::spawn(move || f(seed))
        })
        .collect();

    // Join every thread first — even after we've seen a panic — so no thread
    // is orphaned. Collect results and any panic payloads separately; if any
    // thread panicked, re-raise the first payload on this thread.
    let mut results = Vec::with_capacity(handles.len());
    let mut first_panic = None;
    for h in handles {
        match h.join() {
            Ok(r) => results.push(r),
            Err(payload) => {
                if first_panic.is_none() {
                    first_panic = Some(payload);
                }
            }
        }
    }
    if let Some(payload) = first_panic {
        panic::resume_unwind(payload);
    }
    results
}

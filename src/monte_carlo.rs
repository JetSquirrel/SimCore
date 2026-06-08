//! Monte Carlo helper: run a simulation closure once per seed, in parallel.
//!
//! Two backends are provided, selected at compile time:
//!
//! - **Default (`std::thread`)** — one OS thread is spawned per seed. Simple,
//!   zero extra dependencies; ideal for a modest number of seeds.
//! - **`monte-carlo` feature (`rayon`)** — work is distributed over rayon's
//!   bounded work-stealing thread pool, so the number of live OS threads stays
//!   proportional to the core count rather than the seed count. Preferable when
//!   running hundreds or thousands of seeds.
//!
//! Both backends honour the same contract: results are returned in seed order,
//! and a panic in any worker is re-raised on the calling thread.

/// Run `f` once per seed in parallel and return the results in seed order.
///
/// Each invocation of `f` receives a seed and is responsible for constructing
/// its own [`SimEnv`](crate::env::SimEnv) via
/// [`SimEnv::with_seed`](crate::env::SimEnv::with_seed). Because `SimEnv` is
/// created *inside* the closure it never crosses thread boundaries, so its
/// `!Send` nature is not a problem.
///
/// `F` is shared across threads, so it must be `Send + Sync`. A plain function
/// pointer or a closure that captures only `Send + Sync` data satisfies this
/// automatically.
///
/// # Backend
///
/// With the **`monte-carlo`** feature enabled, the seeds are distributed over
/// rayon's global thread pool (bounded by the core count). Without it, one
/// `std::thread` is spawned per seed. The public contract — seed-ordered
/// results and panic propagation — is identical either way.
///
/// # Panics
///
/// If any worker panics, the original panic payload is re-raised on the calling
/// thread via [`std::panic::resume_unwind`], preserving the original backtrace.
/// With the default backend, surviving threads are still joined before the
/// re-raise, so no threads are orphaned.
pub fn run<F, R>(seeds: impl IntoIterator<Item = u64>, f: F) -> Vec<R>
where
    F: Fn(u64) -> R + Send + Sync + 'static,
    R: Send + 'static,
{
    run_impl(seeds, f)
}

/// rayon backend: distribute seeds over the global work-stealing pool.
///
/// `into_par_iter().map(..).collect()` over an indexed `Vec` preserves order,
/// and rayon re-raises the first worker panic on this thread when `collect`
/// joins — matching the `std::thread` backend's contract.
#[cfg(feature = "monte-carlo")]
fn run_impl<F, R>(seeds: impl IntoIterator<Item = u64>, f: F) -> Vec<R>
where
    F: Fn(u64) -> R + Send + Sync + 'static,
    R: Send + 'static,
{
    use rayon::prelude::*;

    let seeds: Vec<u64> = seeds.into_iter().collect();
    seeds.into_par_iter().map(f).collect()
}

/// Default backend: one OS thread per seed.
#[cfg(not(feature = "monte-carlo"))]
fn run_impl<F, R>(seeds: impl IntoIterator<Item = u64>, f: F) -> Vec<R>
where
    F: Fn(u64) -> R + Send + Sync + 'static,
    R: Send + 'static,
{
    use std::panic;
    use std::sync::Arc;
    use std::thread;

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

// SPDX-FileCopyrightText: 2026 Christoph Kuhmuench <christoph.kuhmuench@gmail.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

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
/// automatically. The closure need **not** be `'static`: the default backend
/// uses [`std::thread::scope`], so `f` (and the results `R`) may borrow from the
/// caller's stack.
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
    F: Fn(u64) -> R + Send + Sync,
    R: Send,
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
    F: Fn(u64) -> R + Send + Sync,
    R: Send,
{
    use rayon::prelude::*;

    let seeds: Vec<u64> = seeds.into_iter().collect();
    seeds.into_par_iter().map(f).collect()
}

/// Default backend: one scoped OS thread per seed.
///
/// Uses [`std::thread::scope`] so `f` and the results can borrow from the
/// caller — no `Arc` wrap and no `'static` bound. The scope joins every thread
/// before returning, so no thread is orphaned even on panic.
#[cfg(not(feature = "monte-carlo"))]
fn run_impl<F, R>(seeds: impl IntoIterator<Item = u64>, f: F) -> Vec<R>
where
    F: Fn(u64) -> R + Send + Sync,
    R: Send,
{
    use std::panic;
    use std::thread;

    let seeds: Vec<u64> = seeds.into_iter().collect();
    let f = &f; // shared by reference across scoped threads (F: Sync ⇒ &F: Send)

    thread::scope(|scope| {
        let handles: Vec<_> = seeds
            .iter()
            .map(|&seed| scope.spawn(move || f(seed)))
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
    })
}

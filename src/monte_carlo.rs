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

    handles.into_iter().map(|h| h.join().unwrap()).collect()
}

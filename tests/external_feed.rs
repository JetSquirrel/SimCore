// SPDX-FileCopyrightText: 2026 Christoph Kuhmuench <christoph.kuhmuench@gmail.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration tests for the external random feed (`SimEnv::with_source`).
//!
//! A `SimEnv` driven by a `SplitMix64` feed must be fully deterministic: two
//! runs with the same seed produce identical traces, different seeds (usually)
//! diverge, and `set_seed` restarts the stream. This is the property the SimPy
//! comparison harness relies on to draw an identical number stream in Python.

use std::cell::RefCell;
use std::rc::Rc;

use simu::SimEnv;
use simu::rng::{sample, SplitMix64};
use simu::{EnvHandle, Resource};

type Log = Rc<RefCell<Vec<String>>>;

/// A small M/M/1-style model: Poisson arrivals into a single-server resource,
/// logging the order and rounded completion time of each job. Every random
/// draw goes through the shared `sample` transforms over the env feed.
fn run_queue(seed: u64, n: u32) -> Vec<String> {
    let mut env = SimEnv::with_source(SplitMix64::new(seed));
    let server = Resource::new(1);
    let log: Log = Rc::new(RefCell::new(Vec::new()));

    env.spawn(arrivals(env.handle(), server, n, log.clone()));
    env.run();

    let out = log.borrow().clone();
    out
}

async fn arrivals(env: EnvHandle, server: Resource, n: u32, log: Log) {
    for id in 0..n {
        let gap = sample::exponential(&mut env.rng(), 1.0 / 0.8);
        env.timeout(gap).await;
        let service = sample::exponential(&mut env.rng(), 1.0);
        env.spawn(job(env.clone(), server.clone(), id, service, log.clone()));
    }
}

async fn job(env: EnvHandle, server: Resource, id: u32, service: f64, log: Log) {
    let _guard = server.request().await;
    env.timeout(service).await;
    log.borrow_mut().push(format!("job{id}@{:.3}", env.now()));
}

#[test]
fn same_seed_produces_identical_trace() {
    let a = run_queue(7, 200);
    let b = run_queue(7, 200);
    assert_eq!(a, b, "same seed must be fully deterministic");
    assert!(!a.is_empty());
}

#[test]
fn different_seeds_diverge() {
    let a = run_queue(1, 200);
    let b = run_queue(2, 200);
    assert_ne!(a, b, "different seeds should produce different traces");
}

#[test]
fn set_seed_restarts_the_stream() {
    // Draw a few values, reseed, and confirm the stream restarts.
    let mut env = SimEnv::with_source(SplitMix64::new(42));
    let first: u64 = {
        use rand::RngCore;
        env.handle().rng().next_u64()
    };
    {
        use rand::RngCore;
        let _ = env.handle().rng().next_u64();
        let _ = env.handle().rng().next_u64();
    }
    env.set_seed(42);
    let after_reseed: u64 = {
        use rand::RngCore;
        env.handle().rng().next_u64()
    };
    assert_eq!(first, after_reseed);
}

#[test]
fn env_feed_matches_standalone_splitmix64() {
    // The env feed and a standalone SplitMix64 must produce the same samples,
    // so a run can be reproduced (or cross-checked) outside the env.
    let env = SimEnv::with_source(SplitMix64::new(123));
    let mut standalone = SplitMix64::new(123);

    for _ in 0..50 {
        let from_env = sample::exponential(&mut env.handle().rng(), 3.0);
        let from_standalone = sample::exponential(&mut standalone, 3.0);
        assert_eq!(from_env, from_standalone);
    }
}

#[test]
fn handles_share_one_feed() {
    // Two handles into the same env share a single feed: draws interleave from
    // one stream rather than each restarting it.
    use rand::RngCore;
    let env = SimEnv::with_source(SplitMix64::new(9));
    let h1 = env.handle();
    let h2 = env.handle();

    // Each draw must release its guard before the next (an RngGuard holds a
    // mutable borrow of the shared feed and cannot overlap with another).
    let a = h1.rng().next_u64();
    let b = h2.rng().next_u64();
    let c = h1.rng().next_u64();

    let mut reference = SplitMix64::new(9);
    assert_eq!(a, reference.next_u64());
    assert_eq!(b, reference.next_u64());
    assert_eq!(c, reference.next_u64());
}

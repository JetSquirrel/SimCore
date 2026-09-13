// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! LLM-provider workload: several agent processes send requests to a shared
//! provider. Two limits apply, and both are plain kernel `Resource`s:
//!
//!  1. **Concurrency cap** — the provider accepts at most
//!     [`PROVIDER_CONCURRENCY`] simultaneous calls; agents queue FIFO for a
//!     slot exactly like cars at the charging-station example.
//!  2. **Token budget (rate limit)** — a token bucket composed *on top of*
//!     the kernel, not baked into it: the bucket is a second `Resource`
//!     whose free units are the available tokens. An agent `request()`s one
//!     unit before each call and parks the guard in a shared `spent` list;
//!     a refill process wakes every [`REFILL_INTERVAL`] and drops one parked
//!     guard, returning that unit (token) to the bucket.
//!
//! The kernel supplies only processes, timeouts, and FIFO resources — quota
//! semantics are assembled entirely at the model layer. Request latency is a
//! plain `env.timeout(...)` with seeded jitter, so the whole run is
//! deterministic: `cargo run --example llm_rate_limit` always prints the
//! same summary.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rand::Rng;
use simcore::{Resource, ResourceGuard, SimEnv};

const NUM_AGENTS: u32 = 3;
const REQUESTS_PER_AGENT: u32 = 5;
const PROVIDER_CONCURRENCY: usize = 2; // simultaneous calls the provider accepts
const BUCKET_CAPACITY: usize = 4; // burst allowance, in tokens
const REFILL_INTERVAL: f64 = 1.0; // one token regenerated per interval
const MEAN_LATENCY: f64 = 2.0; // provider response time, simulated units

const TOTAL_REQUESTS: u32 = NUM_AGENTS * REQUESTS_PER_AGENT;

fn main() {
    let mut env = SimEnv::with_seed(42);

    let provider = Resource::new(PROVIDER_CONCURRENCY);
    let bucket = Resource::new(BUCKET_CAPACITY);

    // Guards of tokens that have been spent but not yet regenerated. Keeping
    // a guard alive is what removes the unit from the bucket; the refill
    // process returns a token simply by dropping one of these.
    let spent: Rc<RefCell<Vec<ResourceGuard>>> = Rc::new(RefCell::new(Vec::new()));

    let completed = Rc::new(Cell::new(0u32));
    let refilled = Rc::new(Cell::new(0u32));

    // --- Agents -------------------------------------------------------------
    for id in 0..NUM_AGENTS {
        let h = env.handle();
        let provider = provider.clone();
        let bucket = bucket.clone();
        let spent = Rc::clone(&spent);
        let completed = Rc::clone(&completed);
        env.spawn(async move {
            for req in 1..=REQUESTS_PER_AGENT {
                // 1. Rate limit: take a token (waits while the bucket is
                //    empty), then park the guard so the unit stays spent.
                let token = bucket.request().await;
                spent.borrow_mut().push(token);

                // 2. Concurrency limit: queue for a provider slot (FIFO).
                let _slot = provider.request().await;

                // 3. Latency: the call itself is simulated time. The RNG is
                //    seeded, so this jitter is reproducible across runs.
                let latency = MEAN_LATENCY * (0.5 + h.rng().random::<f64>());
                h.timeout(latency).await;

                completed.set(completed.get() + 1);
                println!("t={:6.2}  agent {id} completed request {req}", h.now());
                // _slot drops at the end of this iteration → the provider
                // slot is handed to the next queued agent.
            }
        });
    }

    // --- Token-bucket refill ------------------------------------------------
    // The whole "bucket policy" is this one process: every REFILL_INTERVAL it
    // returns at most one spent token to the bucket, until all requests are
    // done. Burst size and refill rate are model-level constants above; no
    // rate-limit logic exists in the kernel.
    {
        let h = env.handle();
        let spent = Rc::clone(&spent);
        let completed = Rc::clone(&completed);
        let refilled = Rc::clone(&refilled);
        env.spawn(async move {
            while completed.get() < TOTAL_REQUESTS {
                h.timeout(REFILL_INTERVAL).await;
                let token = spent.borrow_mut().pop();
                if token.is_some() {
                    refilled.set(refilled.get() + 1);
                }
                drop(token); // guard drop → one unit (token) back in the bucket
            }
        });
    }

    env.run();

    println!("--- summary (seed 42, deterministic) ---");
    println!("requests completed : {}", completed.get());
    println!("tokens refilled    : {}", refilled.get());
    println!(
        "limits             : {PROVIDER_CONCURRENCY} concurrent calls, bucket of {BUCKET_CAPACITY} tokens + 1 per {REFILL_INTERVAL}"
    );
    println!("simulation ended at t = {:.2}", env.now());
}

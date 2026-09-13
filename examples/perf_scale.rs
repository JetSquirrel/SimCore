// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Performance validation harness (not a tutorial example — see
//! `intro_charging_station.rs` for that). Three scenarios from the PoC
//! checklist:
//!
//!  1. **Event throughput** — thousands of processes each awaiting many
//!     timeouts; reports raw events/sec of the event loop.
//!  2. **Resource contention at scale** — 1 000 agents queueing FIFO on a
//!     small resource pool; measures the request/wake path under load.
//!  3. **24h virtual time** — agents waking every virtual minute for a full
//!     simulated day; shows how fast wall-clock time advances virtual time.
//!
//! Every scenario prints a deterministic checksum (sum of per-process
//! completion times) so two runs can be diffed to confirm reproducibility
//! at scale. Run optimized:
//!
//! ```sh
//! cargo run --release --example perf_scale
//! ```

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use rand::Rng;
use simcore::{Resource, SimEnv};

fn main() {
    event_throughput();
    resource_contention();
    virtual_day();
}

/// Scenario 1: raw event-loop throughput.
fn event_throughput() {
    const PROCESSES: u32 = 10_000;
    const TIMEOUTS_EACH: u32 = 100;

    let started = Instant::now();
    let mut env = SimEnv::with_seed(42);
    let checksum = Rc::new(RefCell::new(0.0f64));

    for _ in 0..PROCESSES {
        let h = env.handle();
        let checksum = Rc::clone(&checksum);
        env.spawn(async move {
            let mut sum = 0.0;
            for _ in 0..TIMEOUTS_EACH {
                h.timeout(1.0).await;
                sum += h.now();
            }
            *checksum.borrow_mut() += sum;
        });
    }

    env.run();
    let elapsed = started.elapsed();

    let events = f64::from(PROCESSES) * f64::from(TIMEOUTS_EACH);
    println!("--- scenario 1: event throughput ---");
    println!("processes          : {PROCESSES}");
    println!("events processed   : {events:.0} (timeouts)");
    println!("wall time          : {:.2?}", elapsed);
    println!(
        "throughput         : {:.0} events/sec",
        events / elapsed.as_secs_f64()
    );
    println!("virtual time       : {:.0}", env.now());
    println!("checksum           : {:.6}", checksum.borrow());
    println!();
}

/// Scenario 2: 1 000 agents contending for an 8-slot pool, 20 rounds each.
fn resource_contention() {
    const AGENTS: u32 = 1_000;
    const ROUNDS: u32 = 20;
    const POOL: usize = 8;

    let started = Instant::now();
    let mut env = SimEnv::with_seed(42);
    let pool = Resource::new(POOL);
    let checksum = Rc::new(RefCell::new(0.0f64));

    for _ in 0..AGENTS {
        let h = env.handle();
        let pool = pool.clone();
        let checksum = Rc::clone(&checksum);
        env.spawn(async move {
            for _ in 0..ROUNDS {
                let _slot = pool.request().await; // queue FIFO under contention
                h.timeout(0.1).await; // hold the slot
            }
            *checksum.borrow_mut() += h.now();
        });
    }

    env.run();
    let elapsed = started.elapsed();

    // Each round schedules a slot-grant event plus a timeout event.
    let events = f64::from(AGENTS) * f64::from(ROUNDS) * 2.0;
    println!("--- scenario 2: resource contention (1000 agents, pool of 8) ---");
    println!("events processed   : {events:.0} (grants + timeouts)");
    println!("wall time          : {:.2?}", elapsed);
    println!(
        "throughput         : {:.0} events/sec",
        events / elapsed.as_secs_f64()
    );
    println!("checksum           : {:.6}", checksum.borrow());
    println!();
}

/// Scenario 3: 200 agents waking roughly every virtual minute for a full
/// simulated day, with seeded jitter.
fn virtual_day() {
    const AGENTS: u32 = 200;
    const MINUTES_PER_DAY: u32 = 24 * 60;

    let started = Instant::now();
    let mut env = SimEnv::with_seed(42);
    let checksum = Rc::new(RefCell::new(0.0f64));

    for _ in 0..AGENTS {
        let h = env.handle();
        let checksum = Rc::clone(&checksum);
        env.spawn(async move {
            for _ in 0..MINUTES_PER_DAY {
                // Sleep ~1 virtual minute with seeded jitter, then "work".
                let jitter = h.rng().random::<f64>();
                h.timeout(60.0 * (0.5 + jitter)).await;
            }
            *checksum.borrow_mut() += h.now();
        });
    }

    env.run();
    let elapsed = started.elapsed();

    let events = f64::from(AGENTS) * f64::from(MINUTES_PER_DAY);
    println!("--- scenario 3: 24h virtual time (200 agents, ~1 event/min each) ---");
    println!("events processed   : {events:.0}");
    println!("wall time          : {:.2?}", elapsed);
    println!(
        "throughput         : {:.0} events/sec",
        events / elapsed.as_secs_f64()
    );
    println!(
        "virtual span       : {:.0} s (~{:.1} h)",
        env.now(),
        env.now() / 3600.0
    );
    println!(
        "speedup vs realtime: {:.0}x",
        env.now() / elapsed.as_secs_f64()
    );
    println!("checksum           : {:.6}", checksum.borrow());
}

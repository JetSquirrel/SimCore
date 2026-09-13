// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Toilet queue: people arrive at a restroom with [`NUM_STALLS`] stalls,
//! queue FIFO when every stall is busy, and occupy a stall for a random
//! duration.
//!
//! A single generator process produces arrivals (seeded RNG, so the run is
//! deterministic); each person is its own process that `request()`s a stall,
//! waits in line, and releases it by dropping the guard. The demo tracks how
//! long everyone waited and prints a summary — the classic
//! arrival-rate-vs-service-rate queueing picture, expressed with nothing but
//! `SimEnv`, `Resource`, and `timeout`.

use std::cell::RefCell;
use std::rc::Rc;

use rand::Rng;
use simcore::{Resource, SimEnv};

const NUM_STALLS: usize = 2; // restroom capacity
const NUM_PEOPLE: u32 = 12; // total visitors
const MEAN_INTERARRIVAL: f64 = 2.0; // average time between arrivals
const MEAN_VISIT: f64 = 4.0; // average stall occupancy time

fn main() {
    let mut env = SimEnv::with_seed(42);
    let stalls = Resource::new(NUM_STALLS);

    // Wait time of every person, in arrival order.
    let waits: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(Vec::new()));

    // --- Arrival generator --------------------------------------------------
    // One process that spawns a person every MEAN_INTERARRIVAL * U(0.5, 1.5)
    // time units. With a mean visit twice the mean interarrival time and only
    // two stalls, a queue is guaranteed to build up.
    {
        let h = env.handle();
        let stalls = stalls.clone();
        let waits = Rc::clone(&waits);
        env.spawn(async move {
            for id in 0..NUM_PEOPLE {
                let gap = MEAN_INTERARRIVAL * (0.5 + h.rng().random::<f64>());
                h.timeout(gap).await;

                let person_h = h.clone();
                let stalls = stalls.clone();
                let waits = Rc::clone(&waits);
                h.spawn(async move {
                    let h = person_h;
                    let arrived = h.now();
                    println!("t={:6.2}  person {id:2} arrives", arrived);

                    let _stall = stalls.request().await; // queue FIFO for a stall
                    let wait = h.now() - arrived;
                    waits.borrow_mut().push(wait);
                    println!("t={:6.2}  person {id:2} gets a stall (waited {wait:.2})", h.now());

                    let visit = MEAN_VISIT * (0.5 + h.rng().random::<f64>());
                    h.timeout(visit).await;
                    println!("t={:6.2}  person {id:2} leaves", h.now());
                }); // detached: the generator doesn't wait for each person
            }
        });
    }

    env.run();

    let waits = waits.borrow();
    let total: f64 = waits.iter().sum();
    let max = waits.iter().copied().fold(0.0, f64::max);
    let queued = waits.iter().filter(|&&w| w > 0.0).count();

    println!("--- summary (seed 42, deterministic) ---");
    println!("people served      : {}", waits.len());
    println!("had to queue       : {queued}");
    println!(
        "avg / max wait     : {:.2} / {:.2}",
        total / waits.len() as f64,
        max
    );
    println!(
        "load               : mean visit {MEAN_VISIT} vs mean gap {MEAN_INTERARRIVAL} across {NUM_STALLS} stalls"
    );
    println!("simulation ended at t = {:.2}", env.now());
}

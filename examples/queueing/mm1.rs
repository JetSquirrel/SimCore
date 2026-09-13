// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! M/M/1 queue — the reference example for the queueing validation suite.
//!
//! Poisson arrivals (rate λ), exponential service (rate μ), one server,
//! FCFS. The server is a kernel `Resource` (FIFO by construction); arrivals
//! and services are drawn from the environment's seeded RNG.
//!
//! Analytical results (Harchol-Balter, ch. 13):
//!   ρ    = λ/μ
//!   E[N] = ρ / (1 − ρ)
//!   E[T] = 1 / (μ − λ)
//!
//! Run: `cargo run --release --example queueing-mm1`

#[path = "common/mod.rs"]
mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{check, expovariate, Replications, TimeAvg};
use simcore::{Resource, SimEnv};

const LAMBDA: f64 = 0.8;
const MU: f64 = 1.0;

const REPS: u32 = 12;
const WARMUP: f64 = 5_000.0;
const DURATION: f64 = 100_000.0;

/// One replication. Returns (E[N], E[T], utilization).
fn one_run(seed: u64) -> (f64, f64, f64) {
    let mut env = SimEnv::with_seed(seed);
    let server = Resource::new(1);

    // Time-averaged number in system and server-busy indicator.
    let in_system = Rc::new(RefCell::new(TimeAvg::new(WARMUP)));
    let busy = Rc::new(RefCell::new(TimeAvg::new(WARMUP)));
    // Sum of response times and count, for jobs arriving after warm-up.
    let t_sum = Rc::new(RefCell::new(0.0f64));
    let t_cnt = Rc::new(RefCell::new(0u64));

    {
        let h = env.handle();
        let gen_server = server.clone();
        let gen_in_system = Rc::clone(&in_system);
        let gen_busy = Rc::clone(&busy);
        let gen_t_sum = Rc::clone(&t_sum);
        let gen_t_cnt = Rc::clone(&t_cnt);
        env.spawn(async move {
            loop {
                h.timeout(expovariate(&h, 1.0 / LAMBDA)).await;

                // A new job arrives: one more in the system.
                gen_in_system.borrow_mut().add(h.now(), 1.0);
                let job_h = h.clone();
                let server = gen_server.clone();
                let busy = Rc::clone(&gen_busy);
                let in_system = Rc::clone(&gen_in_system);
                let t_sum = Rc::clone(&gen_t_sum);
                let t_cnt = Rc::clone(&gen_t_cnt);
                h.spawn(async move {
                    let arrived = job_h.now();
                    let _slot = server.request().await; // FCFS queue for the server
                    busy.borrow_mut().add(job_h.now(), 1.0);

                    let service = expovariate(&job_h, 1.0 / MU);
                    job_h.timeout(service).await;
                    drop(_slot);

                    let now = job_h.now();
                    busy.borrow_mut().add(now, -1.0);
                    in_system.borrow_mut().add(now, -1.0);
                    if arrived >= WARMUP {
                        *t_sum.borrow_mut() += now - arrived;
                        *t_cnt.borrow_mut() += 1;
                    }
                });
            }
        });
    }

    let end = WARMUP + DURATION;
    env.run_until(end);

    let e_n = in_system.borrow().mean(end);
    let e_t = *t_sum.borrow() / *t_cnt.borrow() as f64;
    let rho = busy.borrow().mean(end);
    (e_n, e_t, rho)
}

fn main() {
    let rho = LAMBDA / MU;
    println!("M/M/1  lambda={LAMBDA} mu={MU}  (rho={rho})");
    println!("replications: {REPS} x (warmup {WARMUP} + measure {DURATION})\n");
    println!(
        "{:<10} {:>19} {:>12} {:>8}",
        "metric", "measured ± CI95", "theory", "error"
    );

    let mut e_n = Replications::new();
    let mut e_t = Replications::new();
    let mut util = Replications::new();
    for i in 0..REPS {
        let (n, t, r) = one_run(1_000 + u64::from(i));
        e_n.push(n);
        e_t.push(t);
        util.push(r);
    }

    let mut ok = true;
    ok &= check("E[N]", &e_n, rho / (1.0 - rho));
    ok &= check("E[T]", &e_t, 1.0 / (MU - LAMBDA));
    ok &= check("rho", &util, rho);

    // Little's Law cross-check: E[N] should equal lambda * E[T].
    let little = LAMBDA * e_t.mean();
    println!(
        "\nLittle's Law: lambda*E[T] = {little:.4} vs measured E[N] = {:.4}",
        e_n.mean()
    );

    std::process::exit(if ok { 0 } else { 1 });
}

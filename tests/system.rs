// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::cell::RefCell;
use std::rc::Rc;

use rand::RngCore;
use simu::SimEnv;
use simu::Resource;

type Log = Rc<RefCell<Vec<String>>>;
fn new_log() -> Log { Rc::new(RefCell::new(Vec::new())) }

// ---------------------------------------------------------------------------
// Job Shop with Quality Gate
//
// A machine (Resource, capacity 1) processes jobs sequentially. A quality
// inspector fires a gate event after 3 time units; no job may start on the
// machine until the gate is open.
//
// Processes:
//   quality_inspector  — timeout(3) → fire gate → log "gate:3"
//   job_1/2/3          — await gate → acquire machine → timeout(duration) → log "jobN_done:T"
//
// With fixed durations of 2.0 each:
//   t=3:  gate fires; all 3 jobs poll; job1 acquires (job2/3 queue)
//   t=5:  job1 done, drops → job2 acquires
//   t=7:  job2 done, drops → job3 acquires
//   t=9:  job3 done
// ---------------------------------------------------------------------------

/// Build and run the job-shop simulation. Returns (log entries, final time).
///
/// When `rng_durations` is true the job processing times are drawn from the
/// environment's RNG (1–4 time units), making the result seed-dependent.
fn run_job_shop(seed: u64, rng_durations: bool) -> (Vec<String>, f64) {
    let mut env = SimEnv::with_seed(seed);
    let h = env.handle();
    let (trigger, awaitable) = env.event();
    let machine = Resource::new(1);
    let log = new_log();

    // Quality inspector: fires the gate at t=3.
    {
        let h = h.clone();
        let log2 = log.clone();
        env.spawn(async move {
            h.timeout(3.0).await;
            trigger.fire();
            log2.borrow_mut().push(format!("gate:{}", h.now()));
        });
    }

    // Three jobs. Each awaits the gate, then acquires the machine.
    for n in 1..=3_u32 {
        let h = h.clone();
        let machine = machine.clone();
        let aw = awaitable.clone();
        let log2 = log.clone();

        // Optionally sample duration from the RNG; otherwise use fixed 2.0.
        let duration: f64 = if rng_durations {
            // Must be sampled before the async block captures h, because
            // h.rng() returns a guard that borrows from h.
            1.0 + (h.rng().next_u64() % 4) as f64
        } else {
            2.0
        };

        env.spawn(async move {
            aw.await;
            let _guard = machine.request().await;
            h.timeout(duration).await;
            log2.borrow_mut().push(format!("job{}_done:{}", n, h.now()));
            // _guard dropped here, waking the next job
        });
    }

    env.run();
    let final_time = env.now();
    (Rc::try_unwrap(log).unwrap().into_inner(), final_time)
}

#[test]
fn test_system_all_primitives() {
    // Fixed durations of 2.0 each — deterministic trace regardless of seed.
    let (log, final_time) = run_job_shop(42, false);

    assert_eq!(
        log,
        vec!["gate:3", "job1_done:5", "job2_done:7", "job3_done:9"],
    );
    assert_eq!(final_time, 9.0);
}

#[test]
fn test_system_determinism() {
    // With RNG-driven durations the trace is seed-dependent.
    let (log_42a, _) = run_job_shop(42, true);
    let (log_42b, _) = run_job_shop(42, true);
    let (log_99, _)  = run_job_shop(99, true);

    // Same seed → identical trace.
    assert_eq!(log_42a, log_42b, "same seed must produce identical results");

    // Different seeds → different traces (durations differ).
    assert_ne!(log_42a, log_99, "different seeds should produce different results");
}

#[test]
fn monte_carlo_run() {
    // Exercise simu::monte_carlo::run — spawns one thread per seed and
    // collects results in seed order.
    let results = simu::monte_carlo::run(0..4, |seed| {
        let mut env = SimEnv::with_seed(seed);
        let h = env.handle();
        env.spawn(async move { h.timeout(seed as f64).await; });
        env.run();
        env.now()
    });

    // Results are returned in seed order, each equal to its seed value.
    assert_eq!(results, vec![0.0, 1.0, 2.0, 3.0]);
}

#[test]
fn dropping_env_reclaims_suspended_processes() {
    // A process that blocks forever stays suspended in the process table after
    // run() returns. Because the future captures an EnvHandle (which points back
    // at SimState), it forms a reference cycle; dropping the SimEnv must break it
    // so the process — and anything it captured — is reclaimed, not leaked.
    use std::rc::Weak;

    let probe = Rc::new(());
    let weak: Weak<()> = Rc::downgrade(&probe);

    {
        let mut env = SimEnv::with_seed(0);
        let (_trigger, awaitable) = env.event();
        env.spawn(async move {
            let _held = probe; // captured by the suspended future
            awaitable.await; // never fires → process stays suspended
        });
        env.run();
        assert!(weak.upgrade().is_some(), "process should be alive during the run");
    } // env dropped here

    assert!(
        weak.upgrade().is_none(),
        "suspended process leaked: SimState↔process cycle not broken on drop"
    );
}

// --- T4: monte_carlo::run must re-raise a worker panic on the caller ---
// This runs against whichever backend is compiled (std::thread by default,
// rayon under --features monte-carlo); the panic-propagation contract is
// identical for both.

#[test]
#[should_panic(expected = "worker boom")]
fn monte_carlo_propagates_worker_panic() {
    let _ = simu::monte_carlo::run(0..8u64, |seed| {
        if seed == 5 {
            panic!("worker boom");
        }
        seed * 2
    });
}

// --- A5: monte_carlo::run accepts borrowing (non-'static) closures ---

#[test]
fn monte_carlo_accepts_borrowing_closure() {
    // The scoped-thread backend must let the closure borrow caller-stack data
    // (pre-A5 the `'static` bound forced moves/clones). This is primarily a
    // compile-time proof; the assertions confirm the borrowed data was used.
    let offsets: Vec<u64> = vec![100, 200, 300];
    let results = simu::monte_carlo::run(0..3u64, |seed| {
        // `offsets` is borrowed, not moved.
        offsets[seed as usize] + seed
    });
    assert_eq!(results, vec![100, 201, 302]);
    assert_eq!(offsets.len(), 3); // still usable after: proof it was borrowed
}

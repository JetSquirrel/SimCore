use std::cell::RefCell;
use std::rc::Rc;

use simu::env::SimEnv;
use simu::Container;

type Log = Rc<RefCell<Vec<String>>>;
fn new_log() -> Log { Rc::new(RefCell::new(Vec::new())) }

// ---------------------------------------------------------------------------
// Immediate resolution
// ---------------------------------------------------------------------------

#[test]
fn get_immediate_when_level_sufficient() {
    let mut env = SimEnv::with_seed(0);
    let h = env.handle();
    let c = Container::new(10.0, 5.0);
    let log = new_log();

    {
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            c.get(3.0).await;
            log.borrow_mut().push(format!("level:{}", c.level()));
        });
    }

    env.run();
    assert_eq!(*log.borrow(), vec!["level:2"]);
    // Time should be 0 — no suspension occurred.
    assert_eq!(h.now(), 0.0);
}

#[test]
fn put_immediate_when_space_available() {
    let mut env = SimEnv::with_seed(0);
    let h = env.handle();
    let c = Container::empty(10.0);
    let log = new_log();

    {
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            c.put(4.0).await;
            log.borrow_mut().push(format!("level:{}", c.level()));
        });
    }

    env.run();
    assert_eq!(*log.borrow(), vec!["level:4"]);
    assert_eq!(h.now(), 0.0);
}

// ---------------------------------------------------------------------------
// Blocking and waking
// ---------------------------------------------------------------------------

#[test]
fn get_blocks_then_wakes_on_put() {
    let mut env = SimEnv::with_seed(0);
    let c = Container::empty(10.0);
    let log = new_log();

    // Process A: wait 5 time units, then put 3.
    {
        let h = env.handle();
        let c = c.clone();
        env.spawn(async move {
            h.timeout(5.0).await;
            c.put(3.0).await;
        });
    }

    // Process B: immediately try to get 3 — blocks until A's put.
    {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            c.get(3.0).await;
            log.borrow_mut().push(format!("B_got:{}", h.now()));
        });
    }

    env.run();
    assert_eq!(*log.borrow(), vec!["B_got:5"]);
}

#[test]
fn put_blocks_then_wakes_on_get() {
    let mut env = SimEnv::with_seed(0);
    let c = Container::new(5.0, 5.0); // starts full
    let log = new_log();

    // Process A: wait 3 time units, then get 2 (frees space).
    {
        let h = env.handle();
        let c = c.clone();
        env.spawn(async move {
            h.timeout(3.0).await;
            c.get(2.0).await;
        });
    }

    // Process B: immediately try to put 2 — blocks until A's get.
    {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            c.put(2.0).await;
            log.borrow_mut().push(format!("B_put:{}", h.now()));
        });
    }

    env.run();
    assert_eq!(*log.borrow(), vec!["B_put:3"]);
}

// ---------------------------------------------------------------------------
// FIFO ordering
// ---------------------------------------------------------------------------

#[test]
fn fifo_ordering_for_get_waiters() {
    let mut env = SimEnv::with_seed(0);
    let c = Container::empty(10.0);
    let log = new_log();

    // Three consumers each wanting 4 units — all block immediately (level=0).
    for name in ["G1", "G2", "G3"] {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            c.get(4.0).await;
            log.borrow_mut().push(format!("{}:{}", name, h.now()));
        });
    }

    // Producer: supply one batch per time unit.
    {
        let h = env.handle();
        let c = c.clone();
        env.spawn(async move {
            for _ in 0..3 {
                h.timeout(1.0).await;
                c.put(4.0).await;
            }
        });
    }

    env.run();
    assert_eq!(*log.borrow(), vec!["G1:1", "G2:2", "G3:3"]);
}

#[test]
fn fifo_ordering_for_put_waiters() {
    let mut env = SimEnv::with_seed(0);
    let c = Container::new(4.0, 4.0); // starts full
    let log = new_log();

    // Three producers each wanting to put 4 units — all block (no space).
    for name in ["P1", "P2", "P3"] {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            c.put(4.0).await;
            log.borrow_mut().push(format!("{}:{}", name, h.now()));
        });
    }

    // Consumer: drain one batch per time unit.
    {
        let h = env.handle();
        let c = c.clone();
        env.spawn(async move {
            for _ in 0..3 {
                h.timeout(1.0).await;
                c.get(4.0).await;
            }
        });
    }

    env.run();
    assert_eq!(*log.borrow(), vec!["P1:1", "P2:2", "P3:3"]);
}

// Regression: head-of-line FIFO. A freshly-arriving small `get` must NOT take
// level ahead of an already-blocked larger `get`, even when the current level
// would cover the small one. This matches SimPy's Container and the documented
// "served FIFO" contract. (Found by the compare/ SimPy harness: simu used to
// let the small draw bypass the queue.)
//
// Setup: empty cap-10 container.
//   - G_big: get(8) at t=0 -> level 0 < 8 -> blocks (get-waiter #1).
//   - producer puts 5 at t=1: level 0 -> 5. G_big still blocked (5 < 8).
//   - G_small: get(3) arrives at t=2, level is 5 >= 3 but G_big is queued
//     ahead -> G_small must queue behind it, not jump in.
//   - producer puts 5 at t=3: level 5 -> 10. Cascade serves G_big (10 -> 2),
//     then G_small (2 < 3) stays blocked.
//   - producer puts 1 at t=4: level 2 -> 3. Cascade serves G_small.
#[test]
fn fresh_small_get_queues_behind_blocked_large_get() {
    let mut env = SimEnv::with_seed(0);
    let c = Container::empty(10.0);
    let log = new_log();

    // G_big blocks immediately at t=0.
    {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            c.get(8.0).await;
            log.borrow_mut().push(format!("G_big:{}", h.now()));
        });
    }
    // G_small arrives later (t=2), when level (5) already covers its request.
    {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            h.timeout(2.0).await;
            c.get(3.0).await;
            log.borrow_mut().push(format!("G_small:{}", h.now()));
        });
    }
    // Producer drips material: +5 @ t=1, +5 @ t=3, +1 @ t=4.
    {
        let h = env.handle();
        let c = c.clone();
        env.spawn(async move {
            h.timeout(1.0).await;
            c.put(5.0).await;
            h.timeout(2.0).await; // t=3
            c.put(5.0).await;
            h.timeout(1.0).await; // t=4
            c.put(1.0).await;
        });
    }

    env.run();

    // Strict FIFO: G_big served first (at t=3), G_small only after (t=4) —
    // even though at t=2 the level (5) already covered G_small's 3.
    assert_eq!(*log.borrow(), vec!["G_big:3", "G_small:4"]);
}

// Symmetric head-of-line FIFO for the put queue: a fresh small `put` must not
// take free space ahead of an earlier blocked larger `put`.
//
// Setup: cap-10 container starting full (level 10).
//   - P_big: put(8) at t=0 -> 10 + 8 > 10 -> blocks (put-waiter #1).
//   - consumer gets 5 at t=1: level 10 -> 5. P_big still blocked (5 + 8 > 10).
//   - P_small: put(3) arrives at t=2, space is 5 (5 + 3 <= 10) but P_big is
//     queued ahead -> P_small must queue behind it.
//   - consumer gets 5 at t=3: level 5 -> 0. Cascade serves P_big (0 -> 8),
//     then P_small (8 + 3 > 10) stays blocked.
//   - consumer gets 1 at t=4: level 8 -> 7. Cascade serves P_small (7+3=10).
#[test]
fn fresh_small_put_queues_behind_blocked_large_put() {
    let mut env = SimEnv::with_seed(0);
    let c = Container::new(10.0, 10.0); // starts full
    let log = new_log();

    {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            c.put(8.0).await;
            log.borrow_mut().push(format!("P_big:{}", h.now()));
        });
    }
    {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            h.timeout(2.0).await;
            c.put(3.0).await;
            log.borrow_mut().push(format!("P_small:{}", h.now()));
        });
    }
    {
        let h = env.handle();
        let c = c.clone();
        env.spawn(async move {
            h.timeout(1.0).await;
            c.get(5.0).await;
            h.timeout(2.0).await; // t=3
            c.get(5.0).await;
            h.timeout(1.0).await; // t=4
            c.get(1.0).await;
        });
    }

    env.run();

    assert_eq!(*log.borrow(), vec!["P_big:3", "P_small:4"]);
}

// ---------------------------------------------------------------------------
// Cascade
// ---------------------------------------------------------------------------

/// `trigger_cascade` must loop: satisfying a blocked `get` frees space and
/// may unblock a subsequent `put`, whose completion may then unblock another
/// `get`, etc. This test sets up a 4-step chain (`get→put→get→put`) that is
/// resolved by a single `get` at t=1, and verifies every waiter resolves in
/// the same cascade pass (no simulated-time advance).
#[test]
fn cascade_chain_get_put_get_put() {
    let mut env = SimEnv::with_seed(0);
    // Capacity 4, level starts at 4 (full).
    let c = Container::new(4.0, 4.0);
    let log = new_log();

    // Two producers — each blocks immediately (container is full).
    for label in ["P1", "P2"] {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        let label = label.to_string();
        env.spawn(async move {
            c.put(4.0).await;
            log.borrow_mut().push(format!("{}:{}", label, h.now()));
        });
    }

    // Two consumers (beyond the initial 4 units) — G1 blocks at level=0
    // after the initial full level is drained; G2 blocks too.
    // First spawn a consumer that drains the initial level so P1 can fill it.
    {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            c.get(4.0).await; // drains initial full level (resolves immediately)
            log.borrow_mut().push(format!("G_init:{}", h.now()));
        });
    }
    for label in ["G1", "G2"] {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        let label = label.to_string();
        env.spawn(async move {
            c.get(4.0).await;
            log.borrow_mut().push(format!("{}:{}", label, h.now()));
        });
    }

    env.run();

    // Expected cascade at t=0:
    //  1. G_init takes level 4→0 (initial drain)           → wakes P1
    //  2. P1 puts 4, level 0→4                              → wakes G1
    //  3. G1 takes 4, level 4→0                             → wakes P2
    //  4. P2 puts 4, level 0→4                              → wakes G2
    //  5. G2 takes 4, level 4→0                             → end of cascade
    // All five events resolve at t=0 in a single pass of `trigger_cascade`.
    let entries = log.borrow().clone();
    assert_eq!(entries.len(), 5);
    for entry in &entries {
        assert!(
            entry.ends_with(":0"),
            "expected every cascade step at t=0, got: {:?}",
            entries,
        );
    }
    // FIFO order preserved within the get and put queues.
    assert!(entries.contains(&"G_init:0".to_string()));
    assert!(entries.contains(&"P1:0".to_string()));
    assert!(entries.contains(&"P2:0".to_string()));
    assert!(entries.contains(&"G1:0".to_string()));
    assert!(entries.contains(&"G2:0".to_string()));
}

#[test]
fn cascade_satisfies_multiple_gets() {
    let mut env = SimEnv::with_seed(0);
    let c = Container::empty(100.0);
    let log = new_log();

    // Four consumers each wanting 5 units — all block (level=0).
    for i in 1..=4u32 {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            c.get(5.0).await;
            log.borrow_mut().push(format!("G{}:{}", i, h.now()));
        });
    }

    // One large put at t=1 supplies 20 units — should satisfy all four.
    {
        let h = env.handle();
        let c = c.clone();
        env.spawn(async move {
            h.timeout(1.0).await;
            c.put(20.0).await;
        });
    }

    env.run();

    // All four gets resolve at t=1 (same cascade pass).
    assert_eq!(*log.borrow(), vec!["G1:1", "G2:1", "G3:1", "G4:1"]);
}

// Note (strict-FIFO change, 2026-06-28): two earlier regressions
// (`immediate_put_wakes_blocked_put_after_get_drains` and
// `cascade_terminates_on_net_zero_level_delta`, review 2026-06-08 Findings 1 & 2)
// were removed here. Both relied on a fresh *fitting* op of one type slipping
// past an already-blocked *larger* op of the same type to kick a single
// bidirectional cascade pass. That bypass was the FIFO violation fixed in this
// change: a fresh put can no longer take space ahead of an earlier blocked put,
// so those exact setups now deadlock (matching SimPy). The cascade's cross-queue
// waking and work-driven loop are still exercised by `cascade_chain_get_put_get_put`,
// `cascade_satisfies_multiple_gets`, and the put-side mirror below.

// `trigger_cascade` must service *multiple* head-of-queue put-waiters in a single
// pass when one immediate op frees enough space — the put-side mirror of
// `cascade_satisfies_multiple_gets`, and the legal (FIFO-respecting) way to kick a
// cross-queue cascade: an immediate `get` frees space, waking blocked `put`s.
//
// Setup: cap 10, level 10 (full).
//   - P1: put(3) -> 10+3 > 10 -> blocks (put-waiter #1).
//   - P2: put(3) -> blocks (put-waiter #2, FIFO behind P1).
//   - trigger: immediate get(6) at t=1 -> level 10 -> 4 (no get-waiters ahead, so
//     it is legal as an immediate get). The freed space wakes P1 (4+3=7) and then
//     P2 (7+3=10) in one cascade pass.
#[test]
fn immediate_get_cascade_wakes_multiple_blocked_puts() {
    let mut env = SimEnv::with_seed(0);
    let c = Container::new(10.0, 10.0); // starts full
    let log = new_log();

    for name in ["P1", "P2"] {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            c.put(3.0).await;
            log.borrow_mut().push(format!("{}:{}", name, h.now()));
        });
    }
    {
        let h = env.handle();
        let c = c.clone();
        env.spawn(async move {
            h.timeout(1.0).await;
            c.get(6.0).await;
        });
    }

    env.run();

    // Both puts serviced at t=1 in the same cascade pass; FIFO order preserved.
    assert_eq!(*log.borrow(), vec!["P1:1", "P2:1"]);
    // 10 (initial) -6 (get) +3 (P1) +3 (P2) = 10.
    assert_eq!(c.level(), 10.0);
}

// ---------------------------------------------------------------------------
// Accessors
// ---------------------------------------------------------------------------

#[test]
fn level_and_capacity_accessors() {
    let mut env = SimEnv::with_seed(0);
    let c = Container::new(10.0, 3.0);

    assert_eq!(c.capacity(), 10.0);
    assert_eq!(c.level(), 3.0);

    {
        let c = c.clone();
        env.spawn(async move {
            c.put(4.0).await; // level → 7
            c.get(2.0).await; // level → 5
        });
    }

    env.run();
    assert_eq!(c.level(), 5.0);
}

// ---------------------------------------------------------------------------
// Panic tests
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "capacity must be positive")]
fn zero_capacity_panics() {
    let _ = Container::new(0.0, 0.0);
}

#[test]
#[should_panic(expected = "capacity must be positive")]
fn negative_capacity_panics() {
    let _ = Container::new(-1.0, 0.0);
}

#[test]
#[should_panic(expected = "initial_level must not exceed capacity")]
fn initial_level_exceeds_capacity_panics() {
    let _ = Container::new(5.0, 6.0);
}

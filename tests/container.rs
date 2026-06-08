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

// Regression (review 2026-06-08, Finding 1): the immediate-`put` path used to
// call only `wake_get_waiters`, so a get it woke could drain the level and free
// space for a blocked *put*-waiter that was then never woken. The immediate-put
// path must run the full `trigger_cascade`.
//
// Setup: cap 10, level 6.
//   - P_block: put(5) -> 6+5=11 > 10 -> blocks as a put-waiter.
//   - G_block: get(8) -> 6 < 8       -> blocks as a get-waiter.
//   - trigger: an immediate put(2) at t=1 raises level 6 -> 8, which wakes
//     G_block (level 8 -> 0). That frees enough space for P_block (0+5 <= 10),
//     which must now be serviced in the same cascade pass.
#[test]
fn immediate_put_wakes_blocked_put_after_get_drains() {
    let mut env = SimEnv::with_seed(0);
    let c = Container::new(10.0, 6.0);
    let log = new_log();

    {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            c.put(5.0).await;
            log.borrow_mut().push(format!("P_block:{}", h.now()));
        });
    }
    {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            c.get(8.0).await;
            log.borrow_mut().push(format!("G_block:{}", h.now()));
        });
    }
    {
        let h = env.handle();
        let c = c.clone();
        env.spawn(async move {
            h.timeout(1.0).await;
            c.put(2.0).await;
        });
    }

    env.run();

    let entries = log.borrow();
    assert!(
        entries.contains(&"G_block:1".to_string()),
        "get waiter must be served, got: {:?}",
        entries,
    );
    assert!(
        entries.contains(&"P_block:1".to_string()),
        "put waiter must NOT be stranded after the woken get drains the level, got: {:?}",
        entries,
    );
    // 6 (initial) +2 (trigger) -8 (G_block) +5 (P_block) = 5.
    assert_eq!(c.level(), 5.0);
}

// Regression (review 2026-06-08, Finding 2): `trigger_cascade` must keep looping
// while *any* waiter is serviced, even when a pass nets a zero level change.
// Termination must be driven by "work done", never by a float-level delta.
//
// Setup: cap 12, level 5. A blocked get and a blocked put coexist because
// get_amount + put_amount (8 + 8) exceeds capacity, so 4 < level < 8 blocks both.
//   - GA: get(8) -> 5 < 8 blocks (get-waiter #1)
//   - GB: get(8) -> blocks            (get-waiter #2)
//   - PA: put(8) -> 5+8=13 > 12 blocks (put-waiter #1)
//   - trigger: immediate put(3) at t=1 raises level 5 -> 8.
//
// First cascade pass: GA takes 8 (8 -> 0), then PA puts 8 (0 -> 8) — a net-zero
// level change for the pass. A delta-based loop would break here, stranding GB
// even though level is now 8 >= 8. A work-driven loop runs another pass and
// serves GB.
#[test]
fn cascade_terminates_on_net_zero_level_delta() {
    let mut env = SimEnv::with_seed(0);
    let c = Container::new(12.0, 5.0);
    let log = new_log();

    // GA, GB: two get(8) waiters (registered in this order).
    for name in ["GA", "GB"] {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            c.get(8.0).await;
            log.borrow_mut().push(format!("{}:{}", name, h.now()));
        });
    }
    // PA: a put(8) waiter — blocks because 5 + 8 > 12.
    {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            c.put(8.0).await;
            log.borrow_mut().push(format!("PA:{}", h.now()));
        });
    }
    // Trigger: an immediate put(3) at t=1 (5 + 3 = 8 <= 12, so it does not block).
    {
        let h = env.handle();
        let c = c.clone();
        env.spawn(async move {
            h.timeout(1.0).await;
            c.put(3.0).await;
        });
    }

    env.run();

    let entries = log.borrow();
    assert!(entries.contains(&"GA:1".to_string()), "GA must be served: {:?}", entries);
    assert!(entries.contains(&"PA:1".to_string()), "PA must be served: {:?}", entries);
    assert!(
        entries.contains(&"GB:1".to_string()),
        "GB must NOT be stranded by a net-zero cascade pass: {:?}",
        entries,
    );
    // 5 +3 (trigger) -8 (GA) +8 (PA) -8 (GB) = 0.
    assert_eq!(c.level(), 0.0);
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

// SPDX-FileCopyrightText: 2026 Christoph Kuhmuench <christoph.kuhmuench@gmail.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::cell::RefCell;
use std::rc::Rc;

use simu::SimEnv;
use simu::Resource;

type Log = Rc<RefCell<Vec<String>>>;
fn new_log() -> Log { Rc::new(RefCell::new(Vec::new())) }

#[test]
fn acquire_when_capacity_available() {
    let mut env = SimEnv::with_seed(0);
    let resource = Resource::new(2);
    let log = new_log();

    {
        let r = resource.clone();
        let log2 = log.clone();
        env.spawn(async move {
            let _guard = r.request().await;
            log2.borrow_mut().push(format!("in_use:{}", r.in_use()));
            // guard drops here, releasing the unit
        });
    }

    env.run();

    // Process acquired without suspending (no timeout needed).
    assert_eq!(*log.borrow(), vec!["in_use:1"]);
    assert_eq!(resource.in_use(), 0);
    assert_eq!(resource.capacity(), 2);
}

#[test]
fn block_and_wake_on_drop() {
    let mut env = SimEnv::with_seed(0);
    let resource = Resource::new(1);
    let log = new_log();

    // Process A: holds the resource for 5 time units then drops.
    {
        let h = env.handle();
        let r = resource.clone();
        let log2 = log.clone();
        env.spawn(async move {
            let _guard = r.request().await;
            h.timeout(5.0).await;
            log2.borrow_mut().push("A_dropped".to_string());
            // _guard dropped here
        });
    }

    // Process B: immediately tries to acquire — must block until A drops at t=5.
    {
        let h = env.handle();
        let r = resource.clone();
        let log2 = log.clone();
        env.spawn(async move {
            let _guard = r.request().await;
            log2.borrow_mut().push(format!("B_got:{}", h.now()));
        });
    }

    env.run();

    // A drops at t=5, which synchronously wakes B. B is polled next in the
    // same poll_ready pass, so B records t=5.
    assert_eq!(*log.borrow(), vec!["A_dropped", "B_got:5"]);
}

#[test]
fn fifo_ordering_three_waiters() {
    let mut env = SimEnv::with_seed(0);
    let resource = Resource::new(1);
    let log = new_log();

    // Holder: occupies the resource until t=1.
    {
        let h = env.handle();
        let r = resource.clone();
        env.spawn(async move {
            let _guard = r.request().await;
            h.timeout(1.0).await;
            // drops here
        });
    }

    // Three waiters spawned in order W1, W2, W3.
    for label in ["W1", "W2", "W3"] {
        let r = resource.clone();
        let h = env.handle();
        let log2 = log.clone();
        let label = label.to_string();
        env.spawn(async move {
            let _guard = r.request().await;
            log2.borrow_mut().push(label.clone());
            h.timeout(1.0).await;
            // drops here, waking the next waiter
        });
    }

    env.run();

    // FIFO: served in the order they requested.
    assert_eq!(*log.borrow(), vec!["W1", "W2", "W3"]);
}

#[test]
fn guard_drop_releases_exactly_one() {
    // Verify that dropping a guard wakes exactly one waiter, not all of them.
    let mut env = SimEnv::with_seed(0);
    let resource = Resource::new(1);
    let log = new_log();

    // Holder: occupies until t=2, then drops.
    {
        let h = env.handle();
        let r = resource.clone();
        env.spawn(async move {
            let _guard = r.request().await;
            h.timeout(2.0).await;
        });
    }

    // Two waiters. W1 holds the resource past the run_until boundary so W2
    // never gets a turn.
    for label in ["W1", "W2"] {
        let h = env.handle();
        let r = resource.clone();
        let log2 = log.clone();
        let label = label.to_string();
        env.spawn(async move {
            let _guard = r.request().await;
            log2.borrow_mut().push(label);
            h.timeout(100.0).await; // hold well past the run_until boundary
        });
    }

    // Stop at t=3 — enough time for the holder to drop and W1 to run, but
    // W1 holds the resource until t=102 so W2 never acquires it by t=3.
    env.run_until(3.0);

    // Only W1 woke and logged.
    assert_eq!(*log.borrow(), vec!["W1"]);
    assert_eq!(resource.in_use(), 1); // W1 still holds it
}

#[test]
fn in_use_and_capacity_counters() {
    let mut env = SimEnv::with_seed(0);
    let resource = Resource::new(3);

    assert_eq!(resource.in_use(), 0);
    assert_eq!(resource.capacity(), 3);

    let log = new_log();

    for _ in 1..=2_usize {
        let h = env.handle();
        let r = resource.clone();
        let log2 = log.clone();
        env.spawn(async move {
            let _guard = r.request().await;
            // Yield so both guards are held simultaneously before logging.
            // poll_ready acquires for process 1 (in_use=1), then process 2
            // (in_use=2). Both schedule a timeout, then at t=1 each logs
            // the current in_use while the other still holds.
            h.timeout(1.0).await;
            log2.borrow_mut().push(format!("in_use:{}", r.in_use()));
            // _guard dropped here
        });
    }

    env.run();

    // At t=1, both guards are still live when the first process logs.
    // After the first drops, the second sees in_use=1.
    assert!(log.borrow().contains(&"in_use:2".to_string()));
    assert!(log.borrow().contains(&"in_use:1".to_string()));
    assert_eq!(resource.in_use(), 0);
    assert_eq!(resource.capacity(), 3);
}

#[test]
#[should_panic(expected = "capacity must be at least 1")]
fn zero_capacity_panics() {
    let _ = Resource::new(0);
}

// --- A6: queue_len introspection ---

#[test]
fn queue_len_tracks_waiters() {
    let mut env = SimEnv::with_seed(0);
    let r = Resource::new(1);

    // Holder grabs the unit and holds it a while.
    {
        let r = r.clone();
        let h = env.handle();
        env.spawn(async move {
            let _g = r.request().await;
            h.timeout(100.0).await;
        });
    }
    // Two waiters queue up behind it.
    for _ in 0..2 {
        let r = r.clone();
        let h = env.handle();
        env.spawn(async move {
            let _g = r.request().await;
            h.timeout(1.0).await;
        });
    }

    // Run only to t=1 so the holder is still holding and both waiters parked.
    env.run_until(1.0);
    assert_eq!(r.queue_len(), 2, "two processes should be queued");
    assert_eq!(r.in_use(), 1);

    env.run();
    assert_eq!(r.queue_len(), 0, "queue drains by end of run");
}

#[test]
fn resource_debug_is_informative() {
    // A7: Debug impl should render without panicking and mention the type.
    let r = Resource::new(2);
    let s = format!("{r:?}");
    assert!(s.contains("Resource"), "got: {s}");
    assert!(s.contains("capacity"), "got: {s}");
}

use std::cell::RefCell;
use std::rc::Rc;

use simu::env::SimEnv;
use simu::PriorityResource;

type Log = Rc<RefCell<Vec<String>>>;
fn new_log() -> Log { Rc::new(RefCell::new(Vec::new())) }

#[test]
fn acquire_immediately() {
    // When capacity is free, request resolves without suspending.
    let mut env = SimEnv::with_seed(0);
    let resource = PriorityResource::new(2);
    let log = new_log();

    {
        let r = resource.clone();
        let log2 = log.clone();
        env.spawn(async move {
            let _guard = r.request(0).await;
            log2.borrow_mut().push(format!("in_use:{}", r.in_use()));
        });
    }

    env.run();

    assert_eq!(*log.borrow(), vec!["in_use:1"]);
    assert_eq!(resource.in_use(), 0);
    assert_eq!(resource.capacity(), 2);
}

#[test]
fn higher_priority_served_first() {
    // Holder occupies the resource at t=0. Two waiters register while blocked:
    // W_low (priority 1) spawned first, W_high (priority 0) spawned second.
    // When the holder drops, W_high must be served before W_low.
    let mut env = SimEnv::with_seed(0);
    let resource = PriorityResource::new(1);
    let log = new_log();

    // Holder: occupies until t=1.
    {
        let h = env.handle();
        let r = resource.clone();
        env.spawn(async move {
            let _guard = r.request(0).await;
            h.timeout(1.0).await;
        });
    }

    // W_low: spawned first, lower priority.
    {
        let h = env.handle();
        let r = resource.clone();
        let log2 = log.clone();
        env.spawn(async move {
            let _guard = r.request(1).await;
            log2.borrow_mut().push(format!("W_low:{}", h.now()));
            h.timeout(1.0).await;
        });
    }

    // W_high: spawned second, higher priority.
    {
        let h = env.handle();
        let r = resource.clone();
        let log2 = log.clone();
        env.spawn(async move {
            let _guard = r.request(0).await;
            log2.borrow_mut().push(format!("W_high:{}", h.now()));
            h.timeout(1.0).await;
        });
    }

    env.run();

    // W_high woken at t=1, W_low woken at t=2 — priority beats arrival order.
    assert_eq!(*log.borrow(), vec!["W_high:1", "W_low:2"]);
}

#[test]
fn fifo_within_same_priority() {
    // Three waiters all at the same priority level must be served in spawn order.
    let mut env = SimEnv::with_seed(0);
    let resource = PriorityResource::new(1);
    let log = new_log();

    // Holder: occupies until t=1.
    {
        let h = env.handle();
        let r = resource.clone();
        env.spawn(async move {
            let _guard = r.request(0).await;
            h.timeout(1.0).await;
        });
    }

    for label in ["W1", "W2", "W3"] {
        let h = env.handle();
        let r = resource.clone();
        let log2 = log.clone();
        let label = label.to_string();
        env.spawn(async move {
            let _guard = r.request(1).await;
            log2.borrow_mut().push(label);
            h.timeout(1.0).await;
        });
    }

    env.run();

    assert_eq!(*log.borrow(), vec!["W1", "W2", "W3"]);
}

#[test]
fn guard_drop_releases_exactly_one() {
    // Dropping a guard wakes exactly the top-priority waiter, not all.
    let mut env = SimEnv::with_seed(0);
    let resource = PriorityResource::new(1);
    let log = new_log();

    // Holder: occupies until t=2.
    {
        let h = env.handle();
        let r = resource.clone();
        env.spawn(async move {
            let _guard = r.request(0).await;
            h.timeout(2.0).await;
        });
    }

    // Two waiters. Winner holds past the run_until boundary so the loser
    // never gets a turn within the observed window.
    for label in ["W_high", "W_low"] {
        let prio: u32 = if label == "W_high" { 0 } else { 1 };
        let h = env.handle();
        let r = resource.clone();
        let log2 = log.clone();
        let label = label.to_string();
        env.spawn(async move {
            let _guard = r.request(prio).await;
            log2.borrow_mut().push(label);
            h.timeout(100.0).await;
        });
    }

    env.run_until(3.0);

    assert_eq!(*log.borrow(), vec!["W_high"]);
    assert_eq!(resource.in_use(), 1); // W_high still holding
}

#[test]
fn in_use_and_capacity_counters() {
    let mut env = SimEnv::with_seed(0);
    let resource = PriorityResource::new(3);

    assert_eq!(resource.in_use(), 0);
    assert_eq!(resource.capacity(), 3);

    let log = new_log();

    for _ in 0..2 {
        let h = env.handle();
        let r = resource.clone();
        let log2 = log.clone();
        env.spawn(async move {
            let _guard = r.request(0).await;
            h.timeout(1.0).await; // yield so both guards are held simultaneously
            log2.borrow_mut().push(format!("in_use:{}", r.in_use()));
        });
    }

    env.run();

    assert!(log.borrow().contains(&"in_use:2".to_string()));
    assert!(log.borrow().contains(&"in_use:1".to_string()));
    assert_eq!(resource.in_use(), 0);
    assert_eq!(resource.capacity(), 3);
}

#[test]
#[should_panic(expected = "capacity must be at least 1")]
fn zero_capacity_panics() {
    PriorityResource::new(0);
}

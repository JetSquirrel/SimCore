// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Dropped-awaitable safety tests.
//!
//! When a process abandons a suspendable request before it resolves (e.g.,
//! via `any_of!` where a competing sub-future wins first), the primitive's
//! internal waiter queue must not be corrupted: later live waiters must still
//! be woken, and continuous-quantity containers must not leak material.

use std::cell::RefCell;
use std::rc::Rc;

use simcore::SimEnv;
use simcore::{any_of, Container, PriorityResource, Resource};

type Log = Rc<RefCell<Vec<String>>>;
fn new_log() -> Log { Rc::new(RefCell::new(Vec::new())) }

// ---------------------------------------------------------------------------
// Event — dropped awaitable is safe (fire-all-drain semantics)
// ---------------------------------------------------------------------------

#[test]
fn dropped_event_awaitable_does_not_block_others() {
    // Two waiters; one is wrapped in any_of! with a short timeout that wins.
    // The dropped EventAwaitable must not prevent the other from waking when
    // the trigger eventually fires.
    let mut env = SimEnv::with_seed(0);
    let (trigger, awaitable) = env.event();
    let log = new_log();

    // Waiter A: abandons the event after a 1.0 timeout.
    {
        let h = env.handle();
        let aw = awaitable.clone();
        let log = log.clone();
        env.spawn(async move {
            any_of![h.timeout(1.0), aw].await;
            log.borrow_mut().push(format!("A_exits:{}", h.now()));
        });
    }

    // Waiter B: honest await — must wake when the trigger fires.
    {
        let h = env.handle();
        let aw = awaitable;
        let log = log.clone();
        env.spawn(async move {
            aw.await;
            log.borrow_mut().push(format!("B_woken:{}", h.now()));
        });
    }

    // Firer: fires the event at t=5.
    {
        let h = env.handle();
        env.spawn(async move {
            h.timeout(5.0).await;
            trigger.fire();
        });
    }

    env.run();

    let entries = log.borrow();
    assert!(entries.contains(&"A_exits:1".to_string()));
    assert!(entries.contains(&"B_woken:5".to_string()));
}

// ---------------------------------------------------------------------------
// Resource — dropped request behind live waiter
// ---------------------------------------------------------------------------

#[test]
fn dropped_resource_request_does_not_starve_followup() {
    // Setup: capacity 1, holder occupies until t=5.
    //   - W_drop: registers a waiter at t=0, but abandons via any_of! at t=1.
    //   - W_real: registers a waiter at t=0, stays honest.
    // When the holder releases at t=5, W_real must acquire — W_drop must not
    // be incorrectly "served" (its dead waker would cause a lost wake).
    let mut env = SimEnv::with_seed(0);
    let resource = Resource::new(1);
    let log = new_log();

    // Holder: occupies the slot from t=0 to t=5.
    {
        let h = env.handle();
        let r = resource.clone();
        env.spawn(async move {
            let _guard = r.request().await;
            h.timeout(5.0).await;
        });
    }

    // W_drop: abandons the request via any_of at t=1.
    {
        let h = env.handle();
        let r = resource.clone();
        let log = log.clone();
        env.spawn(async move {
            any_of![h.timeout(1.0), async move {
                let _guard = r.request().await;
            }]
            .await;
            log.borrow_mut().push(format!("W_drop_exits:{}", h.now()));
        });
    }

    // W_real: legitimate waiter; must acquire when the holder releases.
    {
        let h = env.handle();
        let r = resource.clone();
        let log = log.clone();
        env.spawn(async move {
            let _guard = r.request().await;
            log.borrow_mut().push(format!("W_real_got:{}", h.now()));
        });
    }

    env.run();

    let entries = log.borrow();
    assert!(entries.contains(&"W_drop_exits:1".to_string()));
    assert!(
        entries.contains(&"W_real_got:5".to_string()),
        "W_real must acquire at t=5 after holder releases, got: {:?}",
        entries,
    );
}

#[test]
fn dropped_priority_resource_request_does_not_starve_followup() {
    let mut env = SimEnv::with_seed(0);
    let resource = PriorityResource::new(1);
    let log = new_log();

    // Holder.
    {
        let h = env.handle();
        let r = resource.clone();
        env.spawn(async move {
            let _guard = r.request(0).await;
            h.timeout(5.0).await;
        });
    }

    // W_drop abandons its priority-1 request.
    {
        let h = env.handle();
        let r = resource.clone();
        let log = log.clone();
        env.spawn(async move {
            any_of![h.timeout(1.0), async move {
                let _guard = r.request(1).await;
            }]
            .await;
            log.borrow_mut().push(format!("W_drop_exits:{}", h.now()));
        });
    }

    // W_real: priority-1, legitimate waiter.
    {
        let h = env.handle();
        let r = resource.clone();
        let log = log.clone();
        env.spawn(async move {
            let _guard = r.request(1).await;
            log.borrow_mut().push(format!("W_real_got:{}", h.now()));
        });
    }

    env.run();

    let entries = log.borrow();
    assert!(entries.contains(&"W_drop_exits:1".to_string()));
    assert!(
        entries.contains(&"W_real_got:5".to_string()),
        "W_real must acquire at t=5, got: {:?}",
        entries,
    );
}

// ---------------------------------------------------------------------------
// Container — dropped request must not deduct level
// ---------------------------------------------------------------------------

#[test]
fn dropped_container_get_does_not_leak_level() {
    // Container level starts at 0. A process tries `get(5)` but abandons at
    // t=1 via any_of with a timeout. Later, a put(5) arrives — the dropped
    // waiter must NOT consume the newly added level.
    let mut env = SimEnv::with_seed(0);
    let c = Container::empty(100.0);
    let log = new_log();

    // Abandons its get after 1 time unit.
    {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            any_of![h.timeout(1.0), c.get(5.0)].await;
            log.borrow_mut().push(format!("dropped_exits:{}", h.now()));
        });
    }

    // Supplier adds 5 units at t=10.
    {
        let h = env.handle();
        let c = c.clone();
        env.spawn(async move {
            h.timeout(10.0).await;
            c.put(5.0).await;
        });
    }

    env.run();

    // After both processes complete, level should equal exactly the put
    // amount (5.0) — the abandoned get must not have deducted anything.
    assert_eq!(
        c.level(),
        5.0,
        "dropped get must not have consumed level; got level={}",
        c.level(),
    );
    assert!(log.borrow().contains(&"dropped_exits:1".to_string()));
}

#[test]
fn dropped_container_put_does_not_add_level() {
    // Container full at capacity=5. A put(3) is issued but abandoned at t=1.
    // Then a get(2) frees space; the dropped put must NOT add its amount.
    let mut env = SimEnv::with_seed(0);
    let c = Container::new(5.0, 5.0);
    let log = new_log();

    {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            any_of![h.timeout(1.0), c.put(3.0)].await;
            log.borrow_mut().push(format!("dropped_exits:{}", h.now()));
        });
    }

    // Consumer removes 2 units at t=10.
    {
        let h = env.handle();
        let c = c.clone();
        env.spawn(async move {
            h.timeout(10.0).await;
            c.get(2.0).await;
        });
    }

    env.run();

    // Level should be 5 - 2 = 3. If the dropped put had added 3, we'd see 6
    // (which would also panic since it exceeds capacity).
    assert_eq!(
        c.level(),
        3.0,
        "dropped put must not have added level; got level={}",
        c.level(),
    );
    assert!(log.borrow().contains(&"dropped_exits:1".to_string()));
}

#[test]
fn dropped_container_get_does_not_starve_followup() {
    // Two consumers both issue get(5); one abandons. A put(5) arrives —
    // the live consumer must be served, not the abandoned one.
    let mut env = SimEnv::with_seed(0);
    let c = Container::empty(100.0);
    let log = new_log();

    // W_drop registers first (head of FIFO queue) and abandons.
    {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            any_of![h.timeout(1.0), c.get(5.0)].await;
            log.borrow_mut().push(format!("W_drop_exits:{}", h.now()));
        });
    }

    // W_real registers after W_drop; must still be served.
    {
        let h = env.handle();
        let c = c.clone();
        let log = log.clone();
        env.spawn(async move {
            c.get(5.0).await;
            log.borrow_mut().push(format!("W_real_got:{}", h.now()));
        });
    }

    // Supplier delivers 5 units at t=10.
    {
        let h = env.handle();
        let c = c.clone();
        env.spawn(async move {
            h.timeout(10.0).await;
            c.put(5.0).await;
        });
    }

    env.run();

    let entries = log.borrow();
    assert!(entries.contains(&"W_drop_exits:1".to_string()));
    assert!(
        entries.contains(&"W_real_got:10".to_string()),
        "W_real must be served at t=10, got: {:?}",
        entries,
    );
}

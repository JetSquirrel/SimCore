//! Regression suite for Finding F1 (see `reviews/2026-07-01-implementation-review.md`):
//! a woken wait-queue waiter must not be stranded — nor jumped in FIFO/priority
//! order — by a *fresh* request that lands in the **same ready batch**.
//!
//! The trigger is a single `EventTrigger::fire()` that wakes both the process
//! about to release a unit and a process about to make a fresh request: they end
//! up in one `poll_ready` batch, so the fresh request is polled before the woken
//! FIFO waiter re-polls. Before the direct-handoff fix the fresh request stole
//! the freed unit; the woken waiter then found itself de-queued with `registered
//! == true` and was stranded forever.
//!
//! These tests would previously deadlock (the assertion process never runs) or
//! violate ordering; they now pass because `WaitQueue::release` hands the unit
//! *directly* to the next live waiter without ever making it observably free.

use std::cell::RefCell;
use std::rc::Rc;

use simu::{any_of, PreemptiveResource, PriorityResource, Resource, SimEnv};

type Log = Rc<RefCell<Vec<String>>>;

fn new_log() -> Log {
    Rc::new(RefCell::new(Vec::new()))
}

/// Plain `Resource`: FIFO waiter A must beat a fresh same-batch requester Q,
/// and — critically — must not be stranded when Q holds the unit across a yield.
#[test]
fn resource_woken_fifo_waiter_beats_same_batch_fresh_request() {
    let mut env = SimEnv::with_seed(0);
    let log = new_log();
    let res = Resource::new(1);
    let (trig, sig) = env.event();

    // P0 holds the unit at t=0, releases inside the broadcast's ready batch (t=1).
    {
        let r = res.clone();
        let h = env.handle();
        let log = log.clone();
        let sig = sig.clone();
        env.spawn(async move {
            let g = r.request().await; // acquires immediately at t=0
            sig.await; // suspend until the broadcast at t=1
            drop(g); // release inside the same ready batch as Q's fresh request
            log.borrow_mut().push(format!("P0 released @{}", h.now()));
        });
    }
    // A: the sole blocked FIFO waiter, queued at t=0.5 (before the release).
    {
        let r = res.clone();
        let h = env.handle();
        let log = log.clone();
        env.spawn(async move {
            h.timeout(0.5).await;
            let _g = r.request().await; // blocks; woken by P0's release
            log.borrow_mut().push(format!("A acquired @{}", h.now()));
            h.timeout(1.0).await; // hold across a yield
        });
    }
    // Q: woken by the SAME broadcast, makes a fresh request in the same batch and
    // holds it across a yield (so a stolen unit would strand A permanently).
    {
        let r = res.clone();
        let h = env.handle();
        let log = log.clone();
        env.spawn(async move {
            sig.await;
            let g = r.request().await;
            log.borrow_mut().push(format!("Q acquired @{}", h.now()));
            h.timeout(1.0).await;
            drop(g);
        });
    }
    // Fire the broadcast at t=1.
    {
        let h = env.handle();
        env.spawn(async move {
            h.timeout(1.0).await;
            trig.fire();
        });
    }

    env.run();

    let log = log.borrow();
    let a_pos = log.iter().position(|l| l.starts_with("A acquired"));
    let q_pos = log.iter().position(|l| l.starts_with("Q acquired"));
    assert!(a_pos.is_some(), "A was stranded (deadlock): {log:?}");
    assert!(q_pos.is_some(), "Q was stranded: {log:?}");
    assert!(
        a_pos < q_pos,
        "FIFO violated: A queued first but Q acquired first: {log:?}"
    );
    assert_eq!(res.in_use(), 0, "unit accounting drifted: {log:?}");
}

/// `PriorityResource`: same-batch steal must not strand a woken waiter, and the
/// higher-priority woken waiter still wins over a same-batch fresh request.
#[test]
fn priority_woken_waiter_beats_same_batch_fresh_request() {
    let mut env = SimEnv::with_seed(0);
    let log = new_log();
    let res = PriorityResource::new(1);
    let (trig, sig) = env.event();

    // P0 holds the unit, releases inside the broadcast batch at t=1.
    {
        let r = res.clone();
        let h = env.handle();
        let log = log.clone();
        let sig = sig.clone();
        env.spawn(async move {
            let g = r.request(0).await;
            sig.await;
            drop(g);
            log.borrow_mut().push(format!("P0 released @{}", h.now()));
        });
    }
    // A: blocked high-priority (0) waiter, queued before the release.
    {
        let r = res.clone();
        let h = env.handle();
        let log = log.clone();
        env.spawn(async move {
            h.timeout(0.5).await;
            let _g = r.request(0).await;
            log.borrow_mut().push(format!("A acquired @{}", h.now()));
            h.timeout(1.0).await;
        });
    }
    // Q: same-batch fresh request, also priority 0, holds across a yield.
    {
        let r = res.clone();
        let h = env.handle();
        let log = log.clone();
        env.spawn(async move {
            sig.await;
            let g = r.request(0).await;
            log.borrow_mut().push(format!("Q acquired @{}", h.now()));
            h.timeout(1.0).await;
            drop(g);
        });
    }
    {
        let h = env.handle();
        env.spawn(async move {
            h.timeout(1.0).await;
            trig.fire();
        });
    }

    env.run();

    let log = log.borrow();
    let a_pos = log.iter().position(|l| l.starts_with("A acquired"));
    let q_pos = log.iter().position(|l| l.starts_with("Q acquired"));
    assert!(a_pos.is_some(), "A was stranded (deadlock): {log:?}");
    assert!(q_pos.is_some(), "Q was stranded: {log:?}");
    assert!(a_pos < q_pos, "priority/FIFO violated: {log:?}");
    assert_eq!(res.in_use(), 0);
}

/// `PreemptiveResource`: the plain blocked-waiter release path also goes through
/// `WaitQueue`, so it is subject to the same stranding bug. Equal priority means
/// no preemption — Q must queue behind A rather than steal the freed unit.
#[test]
fn preemptive_woken_waiter_beats_same_batch_fresh_request() {
    let mut env = SimEnv::with_seed(0);
    let log = new_log();
    let res = PreemptiveResource::new(1);
    let (trig, sig) = env.event();

    // P0 holds the unit at priority 0, releases inside the broadcast batch.
    {
        let r = res.clone();
        let h = env.handle();
        let log = log.clone();
        let sig = sig.clone();
        env.spawn(async move {
            let g = r.request(0).await;
            sig.await;
            drop(g);
            log.borrow_mut().push(format!("P0 released @{}", h.now()));
        });
    }
    // A: blocked equal-priority (0) waiter — cannot preempt P0, so it queues.
    {
        let r = res.clone();
        let h = env.handle();
        let log = log.clone();
        env.spawn(async move {
            h.timeout(0.5).await;
            let _g = r.request(0).await;
            log.borrow_mut().push(format!("A acquired @{}", h.now()));
            h.timeout(1.0).await;
        });
    }
    // Q: same-batch fresh equal-priority request, holds across a yield.
    {
        let r = res.clone();
        let h = env.handle();
        let log = log.clone();
        env.spawn(async move {
            sig.await;
            let g = r.request(0).await;
            log.borrow_mut().push(format!("Q acquired @{}", h.now()));
            h.timeout(1.0).await;
            drop(g);
        });
    }
    {
        let h = env.handle();
        env.spawn(async move {
            h.timeout(1.0).await;
            trig.fire();
        });
    }

    env.run();

    let log = log.borrow();
    let a_pos = log.iter().position(|l| l.starts_with("A acquired"));
    let q_pos = log.iter().position(|l| l.starts_with("Q acquired"));
    assert!(a_pos.is_some(), "A was stranded (deadlock): {log:?}");
    assert!(q_pos.is_some(), "Q was stranded: {log:?}");
    assert!(a_pos < q_pos, "FIFO violated: {log:?}");
    assert_eq!(res.in_use(), 0);
}

/// Consumed-guard-dropped: the woken waiter's request arm is polled *first* on
/// the grant wake, so it consumes the grant into a guard — and then the whole
/// arm (guard included) is dropped when the timeout arm also proves ready at the
/// same tick. The guard's drop must pass the unit on; `in_use` stays consistent.
///
/// (The request arm is listed first in the `any_of!`, so this deliberately does
/// NOT exercise the granted-but-unconsumed drop path — see
/// `resource_granted_but_unconsumed_drop_passes_unit_on` for that one.)
#[test]
fn resource_granted_then_dropped_passes_unit_to_next_waiter() {
    let mut env = SimEnv::with_seed(0);
    let log = new_log();
    let res = Resource::new(1);

    // P0 holds the unit and releases at t=5.
    {
        let r = res.clone();
        let h = env.handle();
        env.spawn(async move {
            let g = r.request().await;
            h.timeout(5.0).await;
            drop(g);
        });
    }
    // A races its resource request against a timeout that fires at exactly t=5 —
    // the same tick P0's release hands A the unit. If the timeout arm wins, A's
    // request future is dropped *after* being granted but *before* consuming.
    {
        let r = res.clone();
        let h = env.handle();
        let log = log.clone();
        env.spawn(async move {
            h.timeout(1.0).await; // queue behind P0
            let hi = h.clone();
            let logi = log.clone();
            any_of![
                async move {
                    let _g = r.request().await;
                    logi.borrow_mut().push(format!("A acquired @{}", hi.now()));
                    // hold briefly if we do win, to expose double-release
                    hi.timeout(1.0).await;
                },
                h.timeout(4.0) // fires at t=5, same tick as P0's release
            ]
            .await;
            log.borrow_mut().push(format!("A raced-out @{}", h.now()));
        });
    }
    // B: a plain FIFO waiter behind A. Whatever happens to A, B must eventually
    // get the unit — either from P0 (if A bailed) or from A's guard drop.
    {
        let r = res.clone();
        let h = env.handle();
        let log = log.clone();
        env.spawn(async move {
            h.timeout(2.0).await; // queue behind A
            let _g = r.request().await;
            log.borrow_mut().push(format!("B acquired @{}", h.now()));
            h.timeout(1.0).await;
        });
    }

    env.run();

    let log = log.borrow();
    assert!(
        log.iter().any(|l| l.starts_with("B acquired")),
        "B was stranded — a granted-then-dropped unit leaked: {log:?}"
    );
    assert_eq!(res.in_use(), 0, "double-release or leak: {log:?}");
}

/// Granted-but-unconsumed drop: the timeout arm is listed FIRST in the
/// `any_of!`, so when the grant wake re-polls the combinator at the same tick,
/// the timeout resolves before the request arm is ever re-polled. The request
/// future is then dropped while `granted == true` and `consumed == false` — the
/// exact window where `ResourceRequest::drop` must call `release()` itself to
/// pass the handed-off unit on. Before the direct-handoff fix this scenario
/// stranded B a different way: the wake was consumed by the canceling waiter A,
/// the unit sat free, and B (still parked) was never woken.
#[test]
fn resource_granted_but_unconsumed_drop_passes_unit_on() {
    let mut env = SimEnv::with_seed(0);
    let log = new_log();
    let res = Resource::new(1);

    // P0 holds the unit and releases at t=5.
    {
        let r = res.clone();
        let h = env.handle();
        env.spawn(async move {
            let g = r.request().await;
            h.timeout(5.0).await;
            drop(g);
        });
    }
    // A: queues behind P0, racing a timeout that fires at exactly t=5 — the same
    // tick P0's release grants A the unit. Timeout arm FIRST: it is polled and
    // resolves before the request arm re-polls, so the granted request is
    // dropped unconsumed.
    {
        let r = res.clone();
        let h = env.handle();
        let log = log.clone();
        env.spawn(async move {
            h.timeout(1.0).await; // queue behind P0
            let hi = h.clone();
            let logi = log.clone();
            any_of![
                h.timeout(4.0), // fires at t=5, same tick as the grant
                async move {
                    let _g = r.request().await;
                    logi.borrow_mut().push(format!("A acquired @{}", hi.now()));
                }
            ]
            .await;
            log.borrow_mut().push(format!("A raced-out @{}", h.now()));
        });
    }
    // B: a live FIFO waiter behind A. The unit granted to A (and abandoned) must
    // be handed on to B — not leaked with `in_use` pinned at capacity.
    {
        let r = res.clone();
        let h = env.handle();
        let log = log.clone();
        env.spawn(async move {
            h.timeout(2.0).await; // queue behind A
            let _g = r.request().await;
            log.borrow_mut().push(format!("B acquired @{}", h.now()));
            h.timeout(1.0).await;
        });
    }

    env.run();

    let log = log.borrow();
    assert!(
        log.iter().any(|l| l.starts_with("A raced-out")),
        "A never resolved its race: {log:?}"
    );
    assert!(
        !log.iter().any(|l| l.starts_with("A acquired")),
        "test setup drifted: A's request arm was re-polled before the timeout \
         arm, so the granted-but-unconsumed drop path was not exercised: {log:?}"
    );
    assert!(
        log.iter().any(|l| l.starts_with("B acquired")),
        "B was stranded — the granted-but-unconsumed unit leaked: {log:?}"
    );
    assert_eq!(res.in_use(), 0, "double-release or leak: {log:?}");
}

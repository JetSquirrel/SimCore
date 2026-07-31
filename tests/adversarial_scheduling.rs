// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Adversarial same-ready-batch scheduling probes (review Finding T2).
//!
//! Most resource tests wake contenders via *separate* timeout events, which the
//! executor fully drains between — so same-batch interleavings (the class that
//! produced the F1 deadlock) go unexercised. These tests deliberately force many
//! processes into a single `poll_ready` batch by having them all await one
//! shared [`EventAwaitable`] that a single `EventTrigger::fire()` releases at the
//! same tick, then probe the orderings that stress the direct-handoff protocol:
//! release-then-request, double-release with multiple waiters, preemption during
//! a batch, and mixed `Container` put/get in one batch.

use std::cell::RefCell;
use std::rc::Rc;

use simu::{Container, EventAwaitable, PreemptiveResource, Resource, SimEnv};

type Log = Rc<RefCell<Vec<String>>>;

fn new_log() -> Log {
    Rc::new(RefCell::new(Vec::new()))
}

/// Spawn `n` processes that each await `sig` (so one `fire()` wakes them all into
/// one ready batch) and then run `body(i)` — an async block built by the caller.
fn spawn_batch<F, Fut>(env: &mut SimEnv, sig: &EventAwaitable, n: u32, body: F)
where
    F: Fn(u32) -> Fut,
    Fut: std::future::Future<Output = ()> + 'static,
{
    for i in 0..n {
        let sig = sig.clone();
        let fut = body(i);
        env.spawn(async move {
            sig.await;
            fut.await;
        });
    }
}

/// Capacity-2 resource: two holders release in the *same* batch while two fresh
/// requesters also arrive in that batch. All four processes must make progress
/// with correct accounting — no unit stolen from a woken waiter, no double free.
#[test]
fn double_release_with_two_waiters_same_batch() {
    let mut env = SimEnv::with_seed(0);
    let log = new_log();
    let res = Resource::new(2);
    let (trig, sig) = env.event();

    // Two holders acquire at t=0, then release together when the batch fires.
    for id in 0..2u32 {
        let r = res.clone();
        let sig = sig.clone();
        let log = log.clone();
        let h = env.handle();
        env.spawn(async move {
            let g = r.request().await;
            sig.await; // all released in one batch at t=1
            drop(g);
            log.borrow_mut().push(format!("H{id} released @{}", h.now()));
        });
    }
    // Two waiters block at t=0.5, before the batch — they must be the ones
    // handed the two freed units, ahead of any fresh same-batch requester.
    for id in 0..2u32 {
        let r = res.clone();
        let log = log.clone();
        let h = env.handle();
        env.spawn(async move {
            h.timeout(0.5).await;
            let _g = r.request().await;
            log.borrow_mut().push(format!("W{id} acquired @{}", h.now()));
            h.timeout(1.0).await;
        });
    }
    // A fresh requester woken by the same batch — must queue behind W0/W1.
    {
        let r = res.clone();
        let sig = sig.clone();
        let log = log.clone();
        let h = env.handle();
        env.spawn(async move {
            sig.await;
            let _g = r.request().await;
            log.borrow_mut().push(format!("F acquired @{}", h.now()));
            h.timeout(1.0).await;
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
    let w0 = log.iter().position(|l| l == "W0 acquired @1");
    let w1 = log.iter().position(|l| l == "W1 acquired @1");
    let f = log.iter().position(|l| l.starts_with("F acquired"));
    assert!(w0.is_some() && w1.is_some(), "a queued waiter was stranded: {log:?}");
    assert!(f.is_some(), "fresh requester never acquired: {log:?}");
    assert!(
        w0 < f && w1 < f,
        "fresh requester jumped ahead of earlier waiters: {log:?}"
    );
    assert_eq!(res.in_use(), 0, "accounting drifted: {log:?}");
}

/// A preemption that lands in the same batch as unrelated wakeups: a high-prio
/// request evicts a low-prio holder while other processes are in the batch. The
/// victim must observe preemption and the preemptor must acquire immediately.
#[test]
fn preempt_during_batch() {
    let mut env = SimEnv::with_seed(0);
    let log = new_log();
    let res = PreemptiveResource::new(1);
    let (trig, sig) = env.event();

    // Low-priority holder acquires at t=0 and would hold for a long time.
    {
        let r = res.clone();
        let log = log.clone();
        let h = env.handle();
        env.spawn(async move {
            let guard = r.request(5).await;
            simu::any_of![h.timeout(100.0), guard.preempted()].await;
            log.borrow_mut().push(format!(
                "victim done @{} preempted={}",
                h.now(),
                guard.is_preempted()
            ));
        });
    }
    // High-priority requester wakes in the batch and preempts the holder.
    {
        let r = res.clone();
        let sig = sig.clone();
        let log = log.clone();
        let h = env.handle();
        env.spawn(async move {
            sig.await;
            let _g = r.request(0).await;
            log.borrow_mut().push(format!("preemptor acquired @{}", h.now()));
            h.timeout(1.0).await;
        });
    }
    // Some unrelated noise processes sharing the same batch.
    spawn_batch(&mut env, &sig, 3, |i| async move {
        let _ = i;
    });
    {
        let h = env.handle();
        env.spawn(async move {
            h.timeout(1.0).await;
            trig.fire();
        });
    }

    env.run();

    let log = log.borrow();
    assert!(
        log.iter().any(|l| l == "preemptor acquired @1"),
        "preemptor did not acquire at the batch tick: {log:?}"
    );
    assert!(
        log.iter().any(|l| l.contains("preempted=true")),
        "victim was not preempted: {log:?}"
    );
    assert_eq!(res.in_use(), 0, "accounting drifted: {log:?}");
}

/// Mixed `Container` put and get arriving in one batch: several gets block, then
/// a batch of puts is released simultaneously. The head-of-line cascade must
/// serve the blocked gets in FIFO order without losing or duplicating level.
#[test]
fn container_mixed_put_get_same_batch() {
    let mut env = SimEnv::with_seed(0);
    let log = new_log();
    let tank = Container::new(100.0, 0.0);
    let (trig, sig) = env.event();

    // Three consumers block immediately (level 0), each wanting 10.
    for id in 0..3u32 {
        let c = tank.clone();
        let log = log.clone();
        let h = env.handle();
        env.spawn(async move {
            c.get(10.0).await;
            log.borrow_mut().push(format!("G{id} @{}", h.now()));
        });
    }
    // Three producers all put 10 in the same batch (released together at t=1).
    for _ in 0..3u32 {
        let c = tank.clone();
        let sig = sig.clone();
        env.spawn(async move {
            sig.await;
            c.put(10.0).await;
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
    // All three gets served, in FIFO order, at the batch tick.
    assert_eq!(*log, vec!["G0 @1", "G1 @1", "G2 @1"], "cascade misbehaved: {log:?}");
    assert_eq!(tank.level(), 0.0, "level drifted: 30 in, 30 out");
    assert_eq!(tank.get_queue_len(), 0);
    assert_eq!(tank.put_queue_len(), 0);
}

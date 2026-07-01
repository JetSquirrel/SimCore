use std::cell::RefCell;
use std::rc::Rc;

use simu::SimEnv;
use simu::{any_of, PreemptiveResource};

type Log = Rc<RefCell<Vec<String>>>;
fn new_log() -> Log {
    Rc::new(RefCell::new(Vec::new()))
}

#[test]
fn acquire_immediately_when_free() {
    // A request against free capacity resolves without suspending or preempting.
    let mut env = SimEnv::with_seed(0);
    let res = PreemptiveResource::new(2);
    let log = new_log();

    {
        let r = res.clone();
        let log2 = log.clone();
        env.spawn(async move {
            let _g = r.request(0).await;
            log2.borrow_mut().push(format!("in_use:{}", r.in_use()));
        });
    }

    env.run();

    assert_eq!(*log.borrow(), vec!["in_use:1"]);
    assert_eq!(res.in_use(), 0);
    assert_eq!(res.capacity(), 2);
}

#[test]
fn higher_priority_preempts_holder_immediately() {
    // Low-priority holder takes the only unit at t=0 and starts a long service.
    // A high-priority request arrives at t=1; it must preempt immediately
    // (NOT wait for the holder's service to finish) and the victim must observe
    // preemption at its yield point.
    let mut env = SimEnv::with_seed(0);
    let res = PreemptiveResource::new(1);
    let log = new_log();

    // Victim: priority 5, races a 100-unit service against preemption.
    {
        let h = env.handle();
        let r = res.clone();
        let log2 = log.clone();
        env.spawn(async move {
            let guard = r.request(5).await;
            log2.borrow_mut().push(format!("victim_start:{}", h.now()));
            any_of![h.timeout(100.0), guard.preempted()].await;
            if guard.is_preempted() {
                log2
                    .borrow_mut()
                    .push(format!("victim_preempted:{}", h.now()));
            } else {
                log2.borrow_mut().push(format!("victim_done:{}", h.now()));
            }
        });
    }

    // Preemptor: arrives at t=1 at higher priority (0).
    {
        let h = env.handle();
        let r = res.clone();
        let log2 = log.clone();
        env.spawn(async move {
            h.timeout(1.0).await;
            let _g = r.request(0).await;
            log2.borrow_mut().push(format!("preemptor_got:{}", h.now()));
            h.timeout(10.0).await;
        });
    }

    env.run();

    // The semantically important facts (independent of how the executor
    // interleaves two events that both occur at t=1):
    //   - the victim observed preemption at t=1, NOT at t=100 (immediate), and
    //   - the preemptor acquired the unit at t=1.
    let log = log.borrow();
    assert_eq!(log[0], "victim_start:0");
    assert!(log.contains(&"victim_preempted:1".to_string()));
    assert!(log.contains(&"preemptor_got:1".to_string()));
    assert!(!log.iter().any(|l| l.starts_with("victim_done")));
    assert_eq!(res.in_use(), 0);
}

#[test]
fn equal_priority_does_not_preempt() {
    // A request at the SAME priority as the holder must not preempt; it waits.
    let mut env = SimEnv::with_seed(0);
    let res = PreemptiveResource::new(1);
    let log = new_log();

    {
        let h = env.handle();
        let r = res.clone();
        let log2 = log.clone();
        env.spawn(async move {
            let guard = r.request(1).await;
            any_of![h.timeout(5.0), guard.preempted()].await;
            log2
                .borrow_mut()
                .push(format!("holder_end:{} preempted:{}", h.now(), guard.is_preempted()));
        });
    }

    {
        let h = env.handle();
        let r = res.clone();
        let log2 = log.clone();
        env.spawn(async move {
            h.timeout(1.0).await;
            let _g = r.request(1).await; // same priority — must queue
            log2.borrow_mut().push(format!("waiter_got:{}", h.now()));
        });
    }

    env.run();

    // Holder runs its full 5 units (never preempted); waiter acquires at t=5.
    assert_eq!(
        *log.borrow(),
        vec!["holder_end:5 preempted:false", "waiter_got:5"]
    );
}

#[test]
fn lower_priority_request_waits_for_release() {
    // Holder at priority 0; a priority-2 request cannot preempt (it is lower
    // priority) and must wait for the holder to release normally.
    let mut env = SimEnv::with_seed(0);
    let res = PreemptiveResource::new(1);
    let log = new_log();

    {
        let h = env.handle();
        let r = res.clone();
        env.spawn(async move {
            let _g = r.request(0).await;
            h.timeout(3.0).await;
        });
    }

    {
        let h = env.handle();
        let r = res.clone();
        let log2 = log.clone();
        env.spawn(async move {
            h.timeout(1.0).await;
            let _g = r.request(2).await;
            log2.borrow_mut().push(format!("low_got:{}", h.now()));
        });
    }

    env.run();

    assert_eq!(*log.borrow(), vec!["low_got:3"]);
}

#[test]
fn preempts_lowest_priority_holder_among_many() {
    // Capacity 2, both units held: one at priority 3, one at priority 8.
    // A priority-1 request must evict the priority-8 holder (the worst), not
    // the priority-3 one.
    let mut env = SimEnv::with_seed(0);
    let res = PreemptiveResource::new(2);
    let log = new_log();

    // Holder A: priority 3.
    {
        let h = env.handle();
        let r = res.clone();
        let log2 = log.clone();
        env.spawn(async move {
            let guard = r.request(3).await;
            any_of![h.timeout(50.0), guard.preempted()].await;
            log2
                .borrow_mut()
                .push(format!("A_end preempted:{}", guard.is_preempted()));
        });
    }

    // Holder B: priority 8 (the eviction target). Spawned after A so both are
    // resident before the preemptor arrives.
    {
        let h = env.handle();
        let r = res.clone();
        let log2 = log.clone();
        env.spawn(async move {
            let guard = r.request(8).await;
            any_of![h.timeout(50.0), guard.preempted()].await;
            log2
                .borrow_mut()
                .push(format!("B_end preempted:{}", guard.is_preempted()));
        });
    }

    // Preemptor: priority 1 at t=1.
    {
        let h = env.handle();
        let r = res.clone();
        let log2 = log.clone();
        env.spawn(async move {
            h.timeout(1.0).await;
            let _g = r.request(1).await;
            log2.borrow_mut().push("preemptor_got".to_string());
            h.timeout(5.0).await;
        });
    }

    env.run();

    let log = log.borrow();
    // B (priority 8) is evicted; A (priority 3) keeps running to completion.
    assert!(log.contains(&"B_end preempted:true".to_string()));
    assert!(log.contains(&"A_end preempted:false".to_string()));
    assert!(log.contains(&"preemptor_got".to_string()));
}

#[test]
fn preemptor_blocks_when_no_victim_available() {
    // Capacity 1 held by a priority-0 holder. A priority-0 request (equal, not
    // higher) has no valid victim, so it must block and only proceed once the
    // holder releases.
    let mut env = SimEnv::with_seed(0);
    let res = PreemptiveResource::new(1);
    let log = new_log();

    {
        let h = env.handle();
        let r = res.clone();
        env.spawn(async move {
            let _g = r.request(0).await;
            h.timeout(4.0).await;
        });
    }

    {
        let h = env.handle();
        let r = res.clone();
        let log2 = log.clone();
        env.spawn(async move {
            h.timeout(1.0).await;
            let _g = r.request(0).await;
            log2.borrow_mut().push(format!("second_got:{}", h.now()));
        });
    }

    env.run();

    assert_eq!(*log.borrow(), vec!["second_got:4"]);
    assert_eq!(res.in_use(), 0);
}

#[test]
fn preempted_guard_drop_does_not_double_release() {
    // After preemption the victim still holds (and later drops) its guard.
    // That drop must be a no-op for capacity accounting — otherwise in_use
    // would underflow or a spurious unit would appear free.
    let mut env = SimEnv::with_seed(0);
    let res = PreemptiveResource::new(1);
    let log = new_log();

    // Victim holds, gets preempted, then keeps "running" past preemption and
    // finally drops its guard — exercising the no-op drop path.
    {
        let h = env.handle();
        let r = res.clone();
        let log2 = log.clone();
        env.spawn(async move {
            let guard = r.request(9).await;
            any_of![h.timeout(100.0), guard.preempted()].await;
            log2
                .borrow_mut()
                .push(format!("victim preempted:{}", guard.is_preempted()));
            // Hold a bit longer, then drop explicitly.
            h.timeout(1.0).await;
            drop(guard);
            log2.borrow_mut().push(format!("victim_dropped in_use:{}", r.in_use()));
        });
    }

    {
        let h = env.handle();
        let r = res.clone();
        let log2 = log.clone();
        env.spawn(async move {
            h.timeout(1.0).await;
            let _g = r.request(0).await;
            h.timeout(10.0).await;
            log2.borrow_mut().push(format!("preemptor_end in_use:{}", r.in_use()));
        });
    }

    env.run();

    let log = log.borrow();
    assert!(log.contains(&"victim preempted:true".to_string()));
    // When the victim drops its already-preempted guard at t=2, the preemptor
    // is still holding its unit, so in_use stays 1 (no double release).
    assert!(log.contains(&"victim_dropped in_use:1".to_string()));
    assert_eq!(res.in_use(), 0);
}

#[test]
fn release_wakes_blocked_waiter_in_priority_order() {
    // No preemption involved: verify the plain blocked-waiter path still serves
    // in priority order (delegated to the shared WaitQueue).
    let mut env = SimEnv::with_seed(0);
    let res = PreemptiveResource::new(1);
    let log = new_log();

    // Holder at priority 0 until t=1.
    {
        let h = env.handle();
        let r = res.clone();
        env.spawn(async move {
            let _g = r.request(0).await;
            h.timeout(1.0).await;
        });
    }
    // Low-priority waiter (spawned first).
    {
        let h = env.handle();
        let r = res.clone();
        let log2 = log.clone();
        env.spawn(async move {
            let _g = r.request(2).await;
            log2.borrow_mut().push(format!("low:{}", h.now()));
            h.timeout(1.0).await;
        });
    }
    // High-priority waiter (spawned second).
    {
        let h = env.handle();
        let r = res.clone();
        let log2 = log.clone();
        env.spawn(async move {
            let _g = r.request(1).await;
            log2.borrow_mut().push(format!("high:{}", h.now()));
            h.timeout(1.0).await;
        });
    }

    env.run();

    // Both blocked behind the holder; when it frees at t=1 the higher-priority
    // (1) waiter is served before the lower-priority (2) one.
    assert_eq!(*log.borrow(), vec!["high:1", "low:2"]);
}

#[test]
fn tie_break_preempts_most_recently_acquired() {
    // Capacity 2, both units held at the SAME priority 5. A priority-0 request
    // must evict the most-recently-acquired holder (B, spawned second), leaving
    // the older holder (A) running — it has made more progress.
    let mut env = SimEnv::with_seed(0);
    let res = PreemptiveResource::new(2);
    let log = new_log();

    // Holder A: acquires first.
    {
        let h = env.handle();
        let r = res.clone();
        let log2 = log.clone();
        env.spawn(async move {
            let guard = r.request(5).await;
            any_of![h.timeout(50.0), guard.preempted()].await;
            log2
                .borrow_mut()
                .push(format!("A preempted:{}", guard.is_preempted()));
        });
    }

    // Holder B: acquires second (same priority) — the tie-break victim.
    {
        let h = env.handle();
        let r = res.clone();
        let log2 = log.clone();
        env.spawn(async move {
            let guard = r.request(5).await;
            any_of![h.timeout(50.0), guard.preempted()].await;
            log2
                .borrow_mut()
                .push(format!("B preempted:{}", guard.is_preempted()));
        });
    }

    // Preemptor at t=1.
    {
        let h = env.handle();
        let r = res.clone();
        env.spawn(async move {
            h.timeout(1.0).await;
            let _g = r.request(0).await;
            h.timeout(5.0).await;
        });
    }

    env.run();

    let log = log.borrow();
    assert!(log.contains(&"B preempted:true".to_string()));
    assert!(log.contains(&"A preempted:false".to_string()));
}

#[test]
#[should_panic(expected = "capacity must be at least 1")]
fn zero_capacity_panics() {
    let _ = PreemptiveResource::new(0);
}

#[test]
fn woken_waiter_losing_same_tick_race_is_not_starved() {
    // Probe: capacity 1. A blocked equal-priority waiter W is woken when the
    // holder releases at t=5 — but a fresh equal-priority request R arrives at
    // the SAME tick and may grab the freed unit first. Whichever loses the race
    // must remain queued and still be served when the winner releases. If a
    // woken-but-lost waiter were dropped from the queue, one of them would
    // never complete.
    let mut env = SimEnv::with_seed(0);
    let res = PreemptiveResource::new(1);
    let log = new_log();

    // Holder: priority 0, releases at t=5.
    {
        let h = env.handle();
        let r = res.clone();
        env.spawn(async move {
            let _g = r.request(0).await;
            h.timeout(5.0).await;
        });
    }
    // W: blocks at t=1 (holder priority 0 is not a valid victim for priority 0).
    {
        let h = env.handle();
        let r = res.clone();
        let log2 = log.clone();
        env.spawn(async move {
            h.timeout(1.0).await;
            let _g = r.request(0).await;
            log2.borrow_mut().push(format!("W:{}", h.now()));
            h.timeout(2.0).await;
        });
    }
    // R: requests at t=5, the same tick the holder releases.
    {
        let h = env.handle();
        let r = res.clone();
        let log2 = log.clone();
        env.spawn(async move {
            h.timeout(5.0).await;
            let _g = r.request(0).await;
            log2.borrow_mut().push(format!("R:{}", h.now()));
            h.timeout(2.0).await;
        });
    }

    env.run();

    // Both W and R must eventually acquire — neither is starved.
    let log = log.borrow();
    assert!(log.iter().any(|l| l.starts_with("W:")), "W starved: {log:?}");
    assert!(log.iter().any(|l| l.starts_with("R:")), "R starved: {log:?}");
    assert_eq!(res.in_use(), 0);
}

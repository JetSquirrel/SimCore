// SPDX-FileCopyrightText: 2026 Christoph Kuhmuench <christoph.kuhmuench@gmail.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use simu::{all_of, any_of, SimEnv};

type Log = Rc<RefCell<Vec<String>>>;
fn new_log() -> Log { Rc::new(RefCell::new(Vec::new())) }

// ---------------------------------------------------------------------------
// Basic value return
// ---------------------------------------------------------------------------

#[test]
fn handle_returns_value() {
    let mut env = SimEnv::with_seed(0);
    let h = env.handle();
    let log = new_log();

    let handle = env.spawn(async move {
        h.timeout(3.0).await;
        42_u32
    });

    let log2 = log.clone();
    env.spawn(async move {
        let value = handle.await;
        log2.borrow_mut().push(format!("got:{}", value));
    });

    env.run();
    assert_eq!(*log.borrow(), vec!["got:42"]);
}

// ---------------------------------------------------------------------------
// Await-order edge cases
// ---------------------------------------------------------------------------

#[test]
fn await_before_completion() {
    let mut env = SimEnv::with_seed(0);
    let h = env.handle();
    let log = new_log();

    let handle = env.spawn(async move {
        h.timeout(10.0).await;
        "done"
    });

    let h2 = env.handle();
    let log2 = log.clone();
    env.spawn(async move {
        let value = handle.await;
        log2.borrow_mut().push(format!("{}:{}", value, h2.now()));
    });

    env.run();
    // Awaiter resolves exactly when the child finishes at t=10.
    assert_eq!(*log.borrow(), vec!["done:10"]);
}

#[test]
fn await_after_completion() {
    // Child finishes at t=1; parent waits until t=5, then awaits — should
    // resolve immediately (in the same poll, no suspension).
    let mut env = SimEnv::with_seed(0);
    let h = env.handle();
    let log = new_log();

    let handle = env.spawn(async move {
        h.timeout(1.0).await;
        "early"
    });

    let h2 = env.handle();
    let log2 = log.clone();
    env.spawn(async move {
        h2.timeout(5.0).await;
        let t_before = h2.now();
        let value = handle.await;
        let t_after = h2.now();
        log2.borrow_mut().push(format!("{}:{}->{}", value, t_before, t_after));
    });

    env.run();
    // Awaiter did not suspend (t_before == t_after == 5).
    assert_eq!(*log.borrow(), vec!["early:5->5"]);
}

// ---------------------------------------------------------------------------
// Drop / detach semantics
// ---------------------------------------------------------------------------

#[test]
fn handle_dropped_detaches_process() {
    // Drop the handle at t=0; child must still run to completion and its
    // observable side-effect (a shared counter) must still happen.
    let mut env = SimEnv::with_seed(0);
    let h = env.handle();
    let counter = Rc::new(Cell::new(0_u32));

    let counter2 = Rc::clone(&counter);
    let handle = env.spawn(async move {
        h.timeout(5.0).await;
        counter2.set(counter2.get() + 1);
    });

    drop(handle);

    env.run();
    assert_eq!(counter.get(), 1, "detached child must still run");
}

#[test]
fn completed_unawaited_drops_value() {
    // Spawn a child that returns a DropCounter. Run the sim (child finishes).
    // Then drop the handle (not awaited). The stored value's Drop must fire.
    let mut env = SimEnv::with_seed(0);
    let h = env.handle();
    let drops = Rc::new(Cell::new(0_u32));

    struct DropCounter(Rc<Cell<u32>>);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    let drops2 = Rc::clone(&drops);
    let handle = env.spawn(async move {
        h.timeout(1.0).await;
        DropCounter(drops2)
    });

    env.run();
    // Child finished; value sits in the slot.
    assert_eq!(drops.get(), 0, "DropCounter not dropped yet (still in slot)");

    drop(handle);
    // Handle drop releases the last Rc to the slot → DropCounter runs Drop.
    assert_eq!(drops.get(), 1, "DropCounter must be dropped with the handle");
}

// ---------------------------------------------------------------------------
// Combinator composition
// ---------------------------------------------------------------------------

#[test]
fn all_of_on_handles() {
    let mut env = SimEnv::with_seed(0);
    let log = new_log();

    let h1 = env.handle();
    let handle_a = env.spawn(async move {
        h1.timeout(2.0).await;
        'a'
    });
    let h2 = env.handle();
    let handle_b = env.spawn(async move {
        h2.timeout(5.0).await;
        'b'
    });

    let h3 = env.handle();
    let log2 = log.clone();
    env.spawn(async move {
        all_of![handle_a.discard(), handle_b.discard()].await;
        log2.borrow_mut().push(format!("all:{}", h3.now()));
    });

    env.run();
    assert_eq!(*log.borrow(), vec!["all:5"]);
}

#[test]
fn any_of_on_handles() {
    let mut env = SimEnv::with_seed(0);
    let log = new_log();

    let h1 = env.handle();
    let handle_a = env.spawn(async move {
        h1.timeout(2.0).await;
    });
    let h2 = env.handle();
    let handle_b = env.spawn(async move {
        h2.timeout(5.0).await;
    });

    let h3 = env.handle();
    let log2 = log.clone();
    env.spawn(async move {
        any_of![handle_a.discard(), handle_b.discard()].await;
        log2.borrow_mut().push(format!("first:{}", h3.now()));
    });

    env.run();
    assert_eq!(*log.borrow(), vec!["first:2"]);
}

// ---------------------------------------------------------------------------
// Nested / cross-process patterns
// ---------------------------------------------------------------------------

#[test]
fn nested_handle() {
    // Outer process spawns an inner one and returns its handle as its
    // own return value. The root awaiter unwraps twice.
    // The `async_yields_async` lint fires because the outer async block
    // yields a Future (the inner handle) — which is exactly what this test
    // verifies is legal and useful.
    #[allow(clippy::async_yields_async)]
    let mut env = SimEnv::with_seed(0);
    let log = new_log();

    let outer_env = env.handle();
    #[allow(clippy::async_yields_async)]
    let outer = env.spawn(async move {
        let inner_env = outer_env.clone();
        let inner = outer_env.spawn(async move {
            inner_env.timeout(3.0).await;
            7_u32
        });
        // Return the inner handle itself.
        inner
    });

    let log2 = log.clone();
    env.spawn(async move {
        let inner = outer.await;    // resolves immediately — outer finishes in poll pass 1
        let value = inner.await;    // suspends until t=3
        log2.borrow_mut().push(format!("{}", value));
    });

    env.run();
    assert_eq!(*log.borrow(), vec!["7"]);
}

#[test]
fn handle_awaited_from_different_process() {
    // Process A spawns process B. Process C (spawned separately) awaits B's
    // handle via a move into an async block.
    let mut env = SimEnv::with_seed(0);
    let log = new_log();

    let a_env = env.handle();
    let log_for_c = log.clone();
    env.spawn(async move {
        let b_env = a_env.clone();
        let b_handle = a_env.spawn(async move {
            b_env.timeout(4.0).await;
            "B done"
        });
        // Hand the handle off to another process via a third spawn.
        let c_env = a_env.clone();
        a_env.spawn(async move {
            let value = b_handle.await;
            log_for_c
                .borrow_mut()
                .push(format!("C received: {} at t={}", value, c_env.now()));
        });
    });

    env.run();
    assert_eq!(*log.borrow(), vec!["C received: B done at t=4"]);
}

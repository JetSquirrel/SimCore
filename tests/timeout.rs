use std::cell::RefCell;
use std::rc::Rc;

use rand::RngCore;
use simu::env::SimEnv;

type Log = Rc<RefCell<Vec<String>>>;
fn new_log() -> Log { Rc::new(RefCell::new(Vec::new())) }

#[test]
fn single_timeout_advances_time() {
    let mut env = SimEnv::with_seed(0);
    let h = env.handle();
    let log = new_log();
    let log2 = log.clone();

    env.spawn(async move {
        h.timeout(10.0).await;
        log2.borrow_mut().push(format!("{}", h.now()));
    });

    env.run();

    assert_eq!(env.now(), 10.0);
    assert_eq!(*log.borrow(), vec!["10"]);
}

#[test]
fn multiple_timeouts_fire_in_time_order() {
    let mut env = SimEnv::with_seed(0);
    let log = new_log();

    for delay in [30.0_f64, 10.0, 20.0] {
        let h = env.handle();
        let log2 = log.clone();
        env.spawn(async move {
            h.timeout(delay).await;
            log2.borrow_mut().push(format!("{}", h.now()));
        });
    }

    env.run();

    assert_eq!(*log.borrow(), vec!["10", "20", "30"]);
}

#[test]
fn run_until_stops_at_boundary() {
    let mut env = SimEnv::with_seed(0);
    let log = new_log();

    for delay in [5.0_f64, 15.0] {
        let h = env.handle();
        let log2 = log.clone();
        env.spawn(async move {
            h.timeout(delay).await;
            log2.borrow_mut().push(format!("{}", h.now()));
        });
    }

    env.run_until(10.0);

    assert_eq!(env.now(), 10.0);
    // Only the t=5 event fired; t=15 is still pending.
    assert_eq!(*log.borrow(), vec!["5"]);
}

#[test]
fn zero_delay_timeout() {
    // timeout(0.0) still schedules a wakeup at the current time and returns
    // Pending on the first poll. It fires in the next event-queue cycle, so
    // env.now() stays at 0.0 but the process does complete.
    let mut env = SimEnv::with_seed(0);
    let h = env.handle();
    let log = new_log();
    let log2 = log.clone();

    env.spawn(async move {
        h.timeout(0.0).await;
        log2.borrow_mut().push(format!("{}", h.now()));
    });

    env.run();

    assert_eq!(env.now(), 0.0);
    assert_eq!(*log.borrow(), vec!["0"]);
}

#[test]
fn deterministic_tie_breaking() {
    // Two processes with the same delay must complete in spawn order because
    // the event queue uses a sequence counter to break ties.
    let mut env = SimEnv::with_seed(0);
    let log = new_log();

    for label in ["A", "B"] {
        let h = env.handle();
        let log2 = log.clone();
        let label = label.to_string();
        env.spawn(async move {
            h.timeout(5.0).await;
            log2.borrow_mut().push(label);
        });
    }

    env.run();

    assert_eq!(*log.borrow(), vec!["A", "B"]);
}

#[test]
fn simenv_new_creates_valid_env() {
    // SimEnv::new() seeds from OS entropy — verify the environment is usable.
    let mut env = SimEnv::new();
    let h = env.handle();
    let log = new_log();
    let log2 = log.clone();

    env.spawn(async move {
        h.timeout(1.0).await;
        log2.borrow_mut().push("done".to_string());
    });

    env.run();

    assert_eq!(env.now(), 1.0);
    assert_eq!(*log.borrow(), vec!["done"]);
}

#[test]
fn simenv_timeout_method() {
    // Exercise SimEnv::timeout() (called on SimEnv, not EnvHandle).
    let mut env = SimEnv::with_seed(0);
    let t = env.timeout(5.0); // called directly on SimEnv
    let log = new_log();
    let log2 = log.clone();
    let h = env.handle();

    env.spawn(async move {
        t.await;
        log2.borrow_mut().push(format!("{}", h.now()));
    });

    env.run();

    assert_eq!(*log.borrow(), vec!["5"]);
}

#[test]
fn rng_guard_covers_all_rngcore_methods() {
    // Exercise next_u32, fill_bytes, and try_fill_bytes on the RNG guard so
    // that all RngCore delegation methods are covered.
    let env = SimEnv::with_seed(0);
    let h = env.handle();

    let _ = h.rng().next_u32();

    let mut buf = [0u8; 8];
    h.rng().fill_bytes(&mut buf);
    assert_ne!(buf, [0u8; 8], "fill_bytes should write non-zero bytes");

    let mut buf2 = [0u8; 8];
    h.rng().try_fill_bytes(&mut buf2).unwrap();
}

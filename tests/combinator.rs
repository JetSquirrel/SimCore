use simu::combinator::{AllOf, AnyOf};
use simu::env::SimEnv;
use simu::{all_of, any_of};

// ---------------------------------------------------------------------------
// AnyOf tests
// ---------------------------------------------------------------------------

#[test]
fn any_of_first_timeout_wins() {
    // Two timeouts: t=1 and t=5. AnyOf must resolve at t=1.
    let mut env = SimEnv::with_seed(0);
    let h = env.handle();

    env.spawn(async move {
        any_of![h.timeout(1.0), h.timeout(5.0)].await;
        assert_eq!(h.now(), 1.0);
    });

    env.run();
}

#[test]
fn any_of_event_beats_timeout() {
    // Timeout at t=10, event fires at t=2. AnyOf resolves at t=2.
    let mut env = SimEnv::with_seed(0);
    let h = env.handle();
    let (trigger, signal) = env.event();

    // Firing process.
    {
        let h2 = h.clone();
        env.spawn(async move {
            h2.timeout(2.0).await;
            trigger.fire();
        });
    }

    env.spawn(async move {
        any_of![h.timeout(10.0), signal].await;
        assert_eq!(h.now(), 2.0);
    });

    env.run();
}

#[test]
fn any_of_already_fired_event() {
    // Event is fired at t=0 before the AnyOf is created. The `fired` latch
    // means the EventAwaitable resolves immediately on first poll (t=0).
    let mut env = SimEnv::with_seed(0);
    let h = env.handle();
    let (trigger, signal) = env.event();
    trigger.fire(); // fires before env.run()

    env.spawn(async move {
        any_of![h.timeout(99.0), signal].await;
        assert_eq!(h.now(), 0.0); // resolved immediately
    });

    env.run();
}

#[test]
#[should_panic(expected = "AnyOf requires at least one future")]
fn any_of_empty_panics() {
    let _fut = AnyOf::new(vec![]);
}

// ---------------------------------------------------------------------------
// AllOf tests
// ---------------------------------------------------------------------------

#[test]
fn all_of_waits_for_last() {
    // Three timeouts: t=1, t=2, t=5. AllOf must resolve at t=5 (the slowest).
    let mut env = SimEnv::with_seed(0);
    let h = env.handle();

    env.spawn(async move {
        all_of![h.timeout(1.0), h.timeout(2.0), h.timeout(5.0)].await;
        assert_eq!(h.now(), 5.0);
    });

    env.run();
}

#[test]
fn all_of_two_events() {
    // Two events: A fires at t=3, B fires at t=7. AllOf resolves at t=7.
    let mut env = SimEnv::with_seed(0);
    let h = env.handle();
    let (trigger_a, event_a) = env.event();
    let (trigger_b, event_b) = env.event();

    {
        let h2 = h.clone();
        env.spawn(async move {
            h2.timeout(3.0).await;
            trigger_a.fire();
        });
    }
    {
        let h2 = h.clone();
        env.spawn(async move {
            h2.timeout(7.0).await;
            trigger_b.fire();
        });
    }

    env.spawn(async move {
        all_of![event_a, event_b].await;
        assert_eq!(h.now(), 7.0);
    });

    env.run();
}

#[test]
fn all_of_empty_resolves_immediately() {
    // AllOf over an empty list resolves immediately (vacuously true).
    let mut env = SimEnv::with_seed(0);
    let h = env.handle();

    env.spawn(async move {
        AllOf::new(vec![]).await;
        assert_eq!(h.now(), 0.0);
    });

    env.run();
}

#[test]
fn all_of_mixed_timeout_and_event() {
    // Timeout at t=5, event fires at t=8. AllOf resolves at t=8.
    let mut env = SimEnv::with_seed(0);
    let h = env.handle();
    let (trigger, signal) = env.event();

    {
        let h2 = h.clone();
        env.spawn(async move {
            h2.timeout(8.0).await;
            trigger.fire();
        });
    }

    env.spawn(async move {
        all_of![h.timeout(5.0), signal].await;
        assert_eq!(h.now(), 8.0);
    });

    env.run();
}

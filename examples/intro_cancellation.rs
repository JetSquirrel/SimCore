// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tutorial chapter 3 (`simcore::tutorial::ch03_events_and_cancellation`): the
//! impatient driver stops a 5-unit charge after 3 units. Manual events plus
//! `any_of!` racing — SimCore's cancellation idiom (SimPy's `Interrupt` analogue).

use simcore::{any_of, SimEnv};

fn main() {
    let mut env = SimEnv::with_seed(42);
    let (stop_charging, stop_signal) = env.event();

    // The car: charge fully — unless told to stop.
    let h = env.handle();
    let car = env.spawn(async move {
        println!("Start charging at {}", h.now());
        any_of![h.timeout(5.0), stop_signal].await;
        println!("Stop charging at {}", h.now());
        h.now() // return when charging actually ended
    });

    // The driver: after 3 time units, wants to leave.
    let h2 = env.handle();
    env.spawn(async move {
        h2.timeout(3.0).await;
        stop_charging.fire(); // wake everyone awaiting the signal
    });

    env.spawn(async move {
        let stopped_at = car.await;
        assert_eq!(stopped_at, 3.0); // the event won the race, not the timeout
        println!("Driver left with a partly charged battery at {stopped_at}");
    });

    env.run();
}

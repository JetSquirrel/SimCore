// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tutorial chapter 2 (`simcore::tutorial::ch02_waiting_for_processes`): an
//! electric car whose driving process waits for its charging process —
//! spawning a child process and awaiting its `ProcessHandle`.

use simcore::SimEnv;

fn main() {
    let mut env = SimEnv::with_seed(42);
    let h = env.handle();

    env.spawn(async move {
        loop {
            println!("Start driving at {}", h.now());
            h.timeout(2.0).await;

            println!("Start charging at {}", h.now());
            let hc = h.clone();
            let charging = h.spawn(async move {
                hc.timeout(5.0).await;
                42.0 // a process can return a value, e.g. the kWh charged
            });
            let kwh = charging.await; // suspend until charging finishes
            assert_eq!(kwh, 42.0);
        }
    });

    env.run_until(15.0);
    println!("Simulation ended at {}", env.now());
}

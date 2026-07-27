// SPDX-FileCopyrightText: 2026 Christoph Kuhmuench <christoph.kuhmuench@gmail.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tutorial chapter 1 (`simu::tutorial::ch01_basic_concepts`): a car that
//! alternately parks and drives. The smallest possible simu model — one
//! process, timeouts, and the clock.

use simu::SimEnv;

fn main() {
    let mut env = SimEnv::with_seed(42);
    let h = env.handle(); // cheap Clone handle, moved into the process

    env.spawn(async move {
        loop {
            println!("Start parking at {}", h.now());
            h.timeout(5.0).await; // park for 5 time units

            println!("Start driving at {}", h.now());
            h.timeout(2.0).await; // drive for 2 time units
        }
    });

    env.run_until(15.0); // drive the event loop until t = 15
    println!("Simulation ended at {}", env.now());
}

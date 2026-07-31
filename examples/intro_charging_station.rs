// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tutorial chapter 4 (`simu::tutorial::ch04_shared_resources`): four cars,
//! staggered arrivals, two charging spots — FIFO queuing on a `Resource` with
//! RAII release.

use simu::{Resource, SimEnv};

fn main() {
    let mut env = SimEnv::with_seed(42);
    let bcs = Resource::new(2); // battery charging station, 2 spots

    for i in 0..4u32 {
        let h = env.handle();
        let station = bcs.clone(); // same pool, cheap Rc clone
        env.spawn(async move {
            h.timeout(f64::from(i) * 2.0).await; // drive to the station
            println!("Car {i} arriving at {}", h.now());

            let _spot = station.request().await; // queue for a spot (FIFO)
            println!("Car {i} starting to charge at {}", h.now());

            h.timeout(5.0).await; // charge
            println!("Car {i} leaving at {}", h.now());
        }); // _spot drops here → spot handed to the next car in line
    }

    env.run();
    println!("Simulation ended at {}", env.now());
}

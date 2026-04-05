use simu::env::{EnvHandle, SimEnv};

/// A single patient: waits until arrival time, then undergoes treatment.
async fn patient(env: EnvHandle, id: u32, arrival: f64, treatment: f64) {
    env.timeout(arrival).await;
    println!("[t={:5.1}] Patient {:2} arrives", env.now(), id);

    env.timeout(treatment).await;
    println!("[t={:5.1}] Patient {:2} discharged", env.now(), id);
}

fn main() {
    let mut env = SimEnv::new();
    let h = env.handle();

    // (id, arrival time, treatment duration) — all times in minutes
    for (id, arrival, treatment) in [
        (1_u32, 0.0_f64, 30.0_f64),
        (2, 5.0, 20.0),
        (3, 10.0, 15.0),
        (4, 15.0, 25.0),
        (5, 20.0, 10.0),
    ] {
        env.spawn(patient(h.clone(), id, arrival, treatment));
    }

    env.run();
    println!("Simulation complete at t={:.1}", env.now());
}

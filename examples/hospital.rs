use simu::env::{EnvHandle, SimEnv};
use simu::event::EventAwaitable;

// ---------------------------------------------------------------------------
// Step 2: triage nurse + manual event
// ---------------------------------------------------------------------------

/// The triage nurse assesses a batch of waiting patients, then fires the
/// event to release them all into the treatment queue simultaneously.
async fn triage_nurse(env: EnvHandle, trigger: simu::event::EventTrigger) {
    println!("[t={:5.1}] Nurse begins triage assessment", env.now());
    env.timeout(10.0).await;
    println!("[t={:5.1}] Nurse completes triage — signalling patients", env.now());
    trigger.fire();
}

/// A patient waits for the triage signal, then undergoes treatment.
async fn patient(
    env: EnvHandle,
    id: u32,
    arrival: f64,
    treatment: f64,
    triage_done: EventAwaitable,
) {
    env.timeout(arrival).await;
    println!("[t={:5.1}] Patient {:2} arrives and waits for triage", env.now(), id);

    triage_done.await;
    println!("[t={:5.1}] Patient {:2} cleared by triage, starting treatment", env.now(), id);

    env.timeout(treatment).await;
    println!("[t={:5.1}] Patient {:2} discharged", env.now(), id);
}

fn main() {
    let mut env = SimEnv::new();
    let h = env.handle();

    // Create the triage event: all patients share the same awaitable.
    let (trigger, triage_done) = env.event();

    // Spawn the nurse (fires at t=10).
    env.spawn(triage_nurse(h.clone(), trigger));

    // Five patients arrive before or around triage completion.
    // (id, arrival time, treatment duration)
    for (id, arrival, treatment) in [
        (1_u32, 0.0_f64, 30.0_f64),
        (2, 5.0, 20.0),
        (3, 10.0, 15.0), // arrives exactly when triage completes
        (4, 15.0, 25.0), // arrives after triage — resolves immediately
        (5, 20.0, 10.0),
    ] {
        env.spawn(patient(h.clone(), id, arrival, treatment, triage_done.clone()));
    }

    env.run();
    println!("Simulation complete at t={:.1}", env.now());
}

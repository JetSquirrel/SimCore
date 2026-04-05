use simu::env::{EnvHandle, SimEnv};
use simu::Resource;

// ---------------------------------------------------------------------------
// Step 3: Resource — nurse serialises triage, beds limit concurrent treatment
// ---------------------------------------------------------------------------

/// A single patient:
///   1. Requests the triage nurse (capacity 1 → serialises assessment).
///   2. Holds the nurse for `triage_duration`, then releases her.
///   3. Requests a bed (capacity 3 → blocks when all occupied).
///   4. Holds the bed for `treatment_duration`, then is discharged.
async fn patient(
    env: EnvHandle,
    id: u32,
    arrival: f64,
    triage_duration: f64,
    treatment_duration: f64,
    nurse: Resource,
    beds: Resource,
) {
    env.timeout(arrival).await;
    println!("[t={:5.1}] Patient {:2} arrives", env.now(), id);

    let _nurse_guard = nurse.request().await;
    println!("[t={:5.1}] Patient {:2} starts triage  (nurse: {}/{})",
        env.now(), id, nurse.in_use(), nurse.capacity());

    env.timeout(triage_duration).await;
    println!("[t={:5.1}] Patient {:2} triage done, awaiting bed", env.now(), id);
    drop(_nurse_guard); // release nurse before waiting for a bed

    let _bed_guard = beds.request().await;
    println!("[t={:5.1}] Patient {:2} admitted to bed  (beds: {}/{})",
        env.now(), id, beds.in_use(), beds.capacity());

    env.timeout(treatment_duration).await;
    println!("[t={:5.1}] Patient {:2} discharged       (beds: {}/{})",
        env.now(), id, beds.in_use() - 1, beds.capacity());
    // _bed_guard dropped here
}

fn main() {
    let mut env = SimEnv::new();
    let h = env.handle();

    let nurse = Resource::new(1); // one triage nurse
    let beds  = Resource::new(3); // three beds

    // (id, arrival, triage_duration, treatment_duration) — times in minutes
    for (id, arrival, triage, treatment) in [
        (1_u32,  0.0_f64, 5.0_f64, 30.0_f64),
        (2,       2.0,    5.0,     20.0),
        (3,       4.0,    5.0,     15.0),
        (4,       6.0,    5.0,     25.0),
        (5,       8.0,    5.0,     10.0),
        (6,      10.0,    5.0,     35.0),
    ] {
        env.spawn(patient(h.clone(), id, arrival, triage, treatment,
                          nurse.clone(), beds.clone()));
    }

    env.run();
    println!("Simulation complete at t={:.1}", env.now());
}

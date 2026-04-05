use rand::Rng;
use rand_distr::Exp;
use simu::env::{EnvHandle, SimEnv};
use simu::Resource;

// ---------------------------------------------------------------------------
// Step 4: Seeded RNG — stochastic arrivals and treatment durations
// ---------------------------------------------------------------------------

const SIM_DURATION: f64 = 480.0; // 8-hour shift in minutes
const ARRIVAL_RATE: f64 = 1.0 / 8.0; // one patient every ~8 minutes
const TRIAGE_DURATION: f64 = 5.0; // nurse takes 5 min per patient (fixed)
const MEAN_TREATMENT: f64 = 20.0; // mean treatment time in minutes

/// Arrival process: spawns a new patient at each Poisson inter-arrival time.
async fn arrivals(env: EnvHandle, nurse: Resource, beds: Resource) {
    let exp = Exp::new(ARRIVAL_RATE).unwrap();
    let mut patient_id = 1_u32;
    loop {
        let inter_arrival = env.rng().sample(exp);
        env.timeout(inter_arrival).await;

        if env.now() > SIM_DURATION {
            break;
        }

        let treatment = env.rng().sample(Exp::new(1.0 / MEAN_TREATMENT).unwrap());
        env.spawn(patient(
            env.clone(),
            patient_id,
            treatment,
            nurse.clone(),
            beds.clone(),
        ));
        patient_id += 1;
    }
    println!("[t={:5.1}] No more arrivals ({} patients total)", env.now(), patient_id - 1);
}

/// A single patient: triage nurse → bed → treatment → discharge.
async fn patient(
    env: EnvHandle,
    id: u32,
    treatment_duration: f64,
    nurse: Resource,
    beds: Resource,
) {
    println!("[t={:5.1}] Patient {:2} arrives", env.now(), id);

    let _nurse_guard = nurse.request().await;
    println!("[t={:5.1}] Patient {:2} starts triage  (nurse: {}/{})",
        env.now(), id, nurse.in_use(), nurse.capacity());

    env.timeout(TRIAGE_DURATION).await;
    println!("[t={:5.1}] Patient {:2} triage done, awaiting bed", env.now(), id);
    drop(_nurse_guard);

    let _bed_guard = beds.request().await;
    println!("[t={:5.1}] Patient {:2} admitted        (beds: {}/{})",
        env.now(), id, beds.in_use(), beds.capacity());

    env.timeout(treatment_duration).await;
    println!("[t={:5.1}] Patient {:2} discharged      (beds: {}/{})",
        env.now(), id, beds.in_use() - 1, beds.capacity());
}

fn main() {
    // Same seed → identical output every run.
    let mut env = SimEnv::with_seed(42);
    let h = env.handle();

    let nurse = Resource::new(1);
    let beds  = Resource::new(3);

    env.spawn(arrivals(h.clone(), nurse, beds));
    env.run();

    println!("Simulation complete at t={:.1}", env.now());
}

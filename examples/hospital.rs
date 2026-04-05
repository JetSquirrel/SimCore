use std::cell::RefCell;
use std::fs::File;
use std::io::Write;
use std::rc::Rc;

use rand::Rng;
use rand_distr::Exp;
use simu::env::{EnvHandle, SimEnv};
use simu::Resource;

// ---------------------------------------------------------------------------
// Step 5: Monte Carlo — 10 parallel runs, per-run log files, summary table
// ---------------------------------------------------------------------------

const SIM_DURATION: f64  = 480.0;        // 8-hour shift in minutes
const ARRIVAL_RATE: f64  = 1.0 / 8.0;   // one patient every ~8 minutes
const TRIAGE_DURATION: f64 = 5.0;        // nurse takes 5 min per patient (fixed)
const MEAN_TREATMENT: f64  = 20.0;       // mean treatment time in minutes

// ---------------------------------------------------------------------------
// Shared state types (Rc<RefCell<>> — safe because the executor is single-threaded)
// ---------------------------------------------------------------------------

type Log   = Rc<RefCell<Vec<String>>>;

struct Stats {
    patients_treated: u32,
    total_nurse_wait: f64,
    total_bed_wait:   f64,
}

type SharedStats = Rc<RefCell<Stats>>;

pub struct SimResult {
    pub seed:             u64,
    pub patients_treated: u32,
    pub mean_nurse_wait:  f64,
    pub mean_bed_wait:    f64,
}

// ---------------------------------------------------------------------------
// Simulation processes
// ---------------------------------------------------------------------------

/// Arrival process: generates patients at Poisson inter-arrival times.
async fn arrivals(
    env:   EnvHandle,
    nurse: Resource,
    beds:  Resource,
    log:   Log,
    stats: SharedStats,
) {
    let exp = Exp::new(ARRIVAL_RATE).unwrap();
    let mut patient_id = 1_u32;

    loop {
        let inter_arrival = env.rng().sample(exp);
        env.timeout(inter_arrival).await;

        if env.now() > SIM_DURATION {
            log.borrow_mut().push(format!(
                "[t={:5.1}] No more arrivals ({} patients total)",
                env.now(), patient_id - 1,
            ));
            break;
        }

        let treatment = env.rng().sample(Exp::new(1.0 / MEAN_TREATMENT).unwrap());
        env.spawn(patient(
            env.clone(), patient_id, treatment,
            nurse.clone(), beds.clone(), log.clone(), stats.clone(),
        ));
        patient_id += 1;
    }
}

/// A single patient: wait for triage nurse → bed → treatment → discharge.
async fn patient(
    env:                EnvHandle,
    id:                 u32,
    treatment_duration: f64,
    nurse:              Resource,
    beds:               Resource,
    log:                Log,
    stats:              SharedStats,
) {
    log.borrow_mut().push(format!("[t={:5.1}] Patient {:2} arrives", env.now(), id));

    let nurse_wait_start = env.now();
    let _nurse = nurse.request().await;
    let nurse_wait = env.now() - nurse_wait_start;
    log.borrow_mut().push(format!(
        "[t={:5.1}] Patient {:2} starts triage  (nurse: {}/{})",
        env.now(), id, nurse.in_use(), nurse.capacity(),
    ));

    env.timeout(TRIAGE_DURATION).await;
    log.borrow_mut().push(format!(
        "[t={:5.1}] Patient {:2} triage done, awaiting bed", env.now(), id,
    ));
    drop(_nurse);

    let bed_wait_start = env.now();
    let _bed = beds.request().await;
    let bed_wait = env.now() - bed_wait_start;
    log.borrow_mut().push(format!(
        "[t={:5.1}] Patient {:2} admitted        (beds: {}/{})",
        env.now(), id, beds.in_use(), beds.capacity(),
    ));

    env.timeout(treatment_duration).await;
    log.borrow_mut().push(format!(
        "[t={:5.1}] Patient {:2} discharged      (beds: {}/{})",
        env.now(), id, beds.in_use() - 1, beds.capacity(),
    ));

    let mut s = stats.borrow_mut();
    s.patients_treated += 1;
    s.total_nurse_wait += nurse_wait;
    s.total_bed_wait   += bed_wait;
}

// ---------------------------------------------------------------------------
// Single-run entry point (called on each OS thread)
// ---------------------------------------------------------------------------

fn run_simulation(seed: u64) -> SimResult {
    let mut env = SimEnv::with_seed(seed);
    let h       = env.handle();

    let nurse = Resource::new(1);
    let beds  = Resource::new(3);
    let log:   Log         = Rc::new(RefCell::new(Vec::new()));
    let stats: SharedStats = Rc::new(RefCell::new(Stats {
        patients_treated: 0,
        total_nurse_wait: 0.0,
        total_bed_wait:   0.0,
    }));

    env.spawn(arrivals(h, nurse, beds, log.clone(), stats.clone()));
    env.run();

    // Write per-run log to its own file.
    let path = format!("run_{:02}.log", seed);
    let mut file = File::create(&path).expect("could not create log file");
    for line in log.borrow().iter() {
        writeln!(file, "{}", line).unwrap();
    }

    let s = stats.borrow();
    let n = s.patients_treated;
    SimResult {
        seed,
        patients_treated: n,
        mean_nurse_wait: if n > 0 { s.total_nurse_wait / n as f64 } else { 0.0 },
        mean_bed_wait:   if n > 0 { s.total_bed_wait   / n as f64 } else { 0.0 },
    }
}

// ---------------------------------------------------------------------------
// Main: run 10 seeds in parallel, print summary table
// ---------------------------------------------------------------------------

fn main() {
    let results = simu::monte_carlo::run(0..10, run_simulation);

    // Header
    println!("{:>6}  {:>8}  {:>18}  {:>15}",
        "Seed", "Patients", "Nurse wait (mean)", "Bed wait (mean)");
    println!("{}", "-".repeat(54));

    for r in &results {
        println!("{:>6}  {:>8}  {:>18.1}  {:>15.1}",
            r.seed, r.patients_treated, r.mean_nurse_wait, r.mean_bed_wait);
    }

    // Aggregate means across all runs
    let n_runs       = results.len() as f64;
    let mean_patients = results.iter().map(|r| r.patients_treated as f64).sum::<f64>() / n_runs;
    let mean_nurse    = results.iter().map(|r| r.mean_nurse_wait).sum::<f64>()          / n_runs;
    let mean_bed      = results.iter().map(|r| r.mean_bed_wait).sum::<f64>()            / n_runs;

    println!("{}", "-".repeat(54));
    println!("{:>6}  {:>8.1}  {:>18.1}  {:>15.1}",
        "mean", mean_patients, mean_nurse, mean_bed);

    println!("\nPer-run logs written to run_00.log … run_09.log");
}

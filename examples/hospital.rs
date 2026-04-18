use std::cell::RefCell;
use std::fs::File;
use std::io::Write;
use std::rc::Rc;

use rand::Rng;
use rand_distr::Exp;
use simu::env::{EnvHandle, SimEnv};
use simu::event::{EventAwaitable, EventTrigger};
use simu::{any_of, PriorityResource, Resource};

// ---------------------------------------------------------------------------
// Hospital simulation — demonstrates post-MVP features:
//
//  • PriorityResource  : nurse serves critical patients (triage 0) first
//  • AnyOf             : treatment races against an early-discharge signal
//                        fired when all beds fill up simultaneously
// ---------------------------------------------------------------------------

const SIM_DURATION: f64    = 480.0;      // 8-hour shift in minutes
const ARRIVAL_RATE: f64    = 1.0 / 8.0; // one patient every ~8 minutes
const TRIAGE_DURATION: f64 = 5.0;       // nurse takes 5 min per patient
const MEAN_TREATMENT: f64  = 20.0;      // mean treatment time in minutes
const CRITICAL_PROB: f64   = 0.3;       // 30 % of patients are critical

// ---------------------------------------------------------------------------
// Shared simulation context
// ---------------------------------------------------------------------------

type Log = Rc<RefCell<Vec<String>>>;

struct Stats {
    critical_treated: u32,
    standard_treated: u32,
    early_discharged: u32,
    total_nurse_wait: f64,
    total_bed_wait:   f64,
}

#[derive(Clone)]
struct HospitalCtx {
    nurse:          PriorityResource,
    beds:           Resource,
    /// Patients await this; fired by the bed-pressure monitor when all beds fill.
    early_discharge: EventAwaitable,
    log:            Log,
    stats:          Rc<RefCell<Stats>>,
}

pub struct SimResult {
    pub seed:             u64,
    pub critical_treated: u32,
    pub standard_treated: u32,
    pub early_discharged: u32,
    pub mean_nurse_wait:  f64,
    pub mean_bed_wait:    f64,
}

// ---------------------------------------------------------------------------
// Simulation processes
// ---------------------------------------------------------------------------

/// Arrival process: generates patients at Poisson inter-arrival times.
async fn arrivals(env: EnvHandle, ctx: HospitalCtx) {
    let exp = Exp::new(ARRIVAL_RATE).unwrap();
    let mut patient_id = 1_u32;

    loop {
        let inter_arrival = env.rng().sample(exp);
        env.timeout(inter_arrival).await;

        if env.now() > SIM_DURATION {
            ctx.log.borrow_mut().push(format!(
                "[t={:5.1}] No more arrivals ({} patients total)",
                env.now(), patient_id - 1,
            ));
            break;
        }

        let treatment   = env.rng().sample(Exp::new(1.0 / MEAN_TREATMENT).unwrap());
        let is_critical = env.rng().gen::<f64>() < CRITICAL_PROB;
        let triage      = if is_critical { 0_u32 } else { 1_u32 };

        env.spawn(patient(env.clone(), patient_id, triage, treatment, ctx.clone()));
        patient_id += 1;
    }
}

/// Bed-pressure monitor: fired once when every bed is simultaneously occupied.
/// Stable (standard) patients in beds will be prompted to leave early.
async fn bed_pressure_monitor(env: EnvHandle, ctx: HospitalCtx, trigger: EventTrigger) {
    loop {
        env.timeout(1.0).await;
        if env.now() > SIM_DURATION { break; }

        if ctx.beds.in_use() >= ctx.beds.capacity() {
            ctx.log.borrow_mut().push(format!(
                "[t={:5.1}] *** BED PRESSURE: all {}/{} beds occupied — early-discharge signal ***",
                env.now(), ctx.beds.in_use(), ctx.beds.capacity(),
            ));
            trigger.fire(); // one-shot; EventTrigger is consumed here
            break;
        }
    }
}

/// A single patient: triage nurse → bed → treatment (or early discharge).
async fn patient(
    env: EnvHandle,
    id: u32,
    triage: u32,
    treatment_duration: f64,
    ctx: HospitalCtx,
) {
    let label = if triage == 0 { "CRITICAL" } else { "standard" };
    ctx.log.borrow_mut().push(format!(
        "[t={:5.1}] Patient {:2} arrives  [{}]", env.now(), id, label,
    ));

    // --- Triage nurse (priority-scheduled) ---
    let nurse_wait_start = env.now();
    let _nurse = ctx.nurse.request(triage).await;
    let nurse_wait = env.now() - nurse_wait_start;
    ctx.log.borrow_mut().push(format!(
        "[t={:5.1}] Patient {:2} starts triage  [{}]  (waited {:.1} min)",
        env.now(), id, label, nurse_wait,
    ));
    env.timeout(TRIAGE_DURATION).await;
    drop(_nurse);

    // --- Bed ---
    let bed_wait_start = env.now();
    let _bed = ctx.beds.request().await;
    let bed_wait = env.now() - bed_wait_start;
    let admitted_at = env.now();
    ctx.log.borrow_mut().push(format!(
        "[t={:5.1}] Patient {:2} admitted        (beds: {}/{})",
        env.now(), id, ctx.beds.in_use(), ctx.beds.capacity(),
    ));

    // --- Treatment races against early-discharge signal ---
    // If the early-discharge event fires before treatment completes, the
    // patient leaves early. Detection: if env.now() < admitted_at + treatment,
    // the signal (not the timeout) caused the wakeup.
    any_of![
        env.timeout(treatment_duration),
        ctx.early_discharge.clone()
    ]
    .await;

    let was_early = env.now() < admitted_at + treatment_duration;
    if was_early {
        ctx.log.borrow_mut().push(format!(
            "[t={:5.1}] Patient {:2} EARLY discharge  (beds: {}/{})",
            env.now(), id, ctx.beds.in_use() - 1, ctx.beds.capacity(),
        ));
    } else {
        ctx.log.borrow_mut().push(format!(
            "[t={:5.1}] Patient {:2} discharged       (beds: {}/{})",
            env.now(), id, ctx.beds.in_use() - 1, ctx.beds.capacity(),
        ));
    }

    let mut s = ctx.stats.borrow_mut();
    if triage == 0 { s.critical_treated += 1; } else { s.standard_treated += 1; }
    if was_early   { s.early_discharged += 1; }
    s.total_nurse_wait += nurse_wait;
    s.total_bed_wait   += bed_wait;
}

// ---------------------------------------------------------------------------
// Single-run entry point
// ---------------------------------------------------------------------------

fn run_simulation(seed: u64) -> SimResult {
    let mut env = SimEnv::with_seed(seed);
    let h = env.handle();

    let (discharge_trigger, discharge_signal) = env.event();

    let ctx = HospitalCtx {
        nurse:           PriorityResource::new(1),
        beds:            Resource::new(3),
        early_discharge: discharge_signal,
        log:             Rc::new(RefCell::new(Vec::new())),
        stats:           Rc::new(RefCell::new(Stats {
            critical_treated: 0,
            standard_treated: 0,
            early_discharged: 0,
            total_nurse_wait: 0.0,
            total_bed_wait:   0.0,
        })),
    };

    env.spawn(arrivals(h.clone(), ctx.clone()));
    env.spawn(bed_pressure_monitor(h, ctx.clone(), discharge_trigger));
    env.run();

    let path = format!("run_{:02}.log", seed);
    let mut file = File::create(&path).expect("could not create log file");
    for line in ctx.log.borrow().iter() {
        writeln!(file, "{}", line).unwrap();
    }

    let s = ctx.stats.borrow();
    let total = s.critical_treated + s.standard_treated;
    SimResult {
        seed,
        critical_treated: s.critical_treated,
        standard_treated: s.standard_treated,
        early_discharged: s.early_discharged,
        mean_nurse_wait: if total > 0 { s.total_nurse_wait / total as f64 } else { 0.0 },
        mean_bed_wait:   if total > 0 { s.total_bed_wait   / total as f64 } else { 0.0 },
    }
}

// ---------------------------------------------------------------------------
// Main: run 10 seeds in parallel, print summary table
// ---------------------------------------------------------------------------

fn main() {
    let results = simu::monte_carlo::run(0..10, run_simulation);

    println!(
        "{:>6}  {:>8}  {:>8}  {:>7}  {:>18}  {:>15}",
        "Seed", "Critical", "Standard", "Early", "Nurse wait (mean)", "Bed wait (mean)"
    );
    println!("{}", "-".repeat(74));

    for r in &results {
        println!(
            "{:>6}  {:>8}  {:>8}  {:>7}  {:>18.1}  {:>15.1}",
            r.seed, r.critical_treated, r.standard_treated, r.early_discharged,
            r.mean_nurse_wait, r.mean_bed_wait,
        );
    }

    let n = results.len() as f64;
    let mean_crit  = results.iter().map(|r| r.critical_treated as f64).sum::<f64>() / n;
    let mean_std   = results.iter().map(|r| r.standard_treated as f64).sum::<f64>() / n;
    let mean_early = results.iter().map(|r| r.early_discharged as f64).sum::<f64>() / n;
    let mean_nurse = results.iter().map(|r| r.mean_nurse_wait).sum::<f64>() / n;
    let mean_bed   = results.iter().map(|r| r.mean_bed_wait).sum::<f64>()   / n;

    println!("{}", "-".repeat(74));
    println!(
        "{:>6}  {:>8.1}  {:>8.1}  {:>7.1}  {:>18.1}  {:>15.1}",
        "mean", mean_crit, mean_std, mean_early, mean_nurse, mean_bed,
    );

    println!("\nPer-run logs written to run_00.log … run_09.log");
}

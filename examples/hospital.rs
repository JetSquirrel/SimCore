use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::rc::Rc;

use rand::Rng;
use rand_distr::Exp;
use simu::env::{EnvHandle, SimEnv};
use simu::event::EventTrigger;
use simu::{any_of, Container, PriorityResource, Resource};

// ---------------------------------------------------------------------------
// Hospital simulation — demonstrates post-MVP features:
//
//  • PriorityResource  : nurse serves critical patients (triage 0) first
//  • AnyOf             : treatment races against a personal eviction signal
//  • Container         : blood bank — critical patients draw 10 units,
//                        standard patients draw 2; restocked every 60 min
//
// When a critical patient arrives and every bed is occupied, they fire the
// eviction trigger of the longest-admitted standard patient, who then
// resolves their any_of early and vacates their bed.
// ---------------------------------------------------------------------------

const SIM_DURATION:     f64 = 480.0;      // 8-hour shift in minutes
const ARRIVAL_RATE:     f64 = 1.0 / 8.0; // one patient every ~8 minutes
const TRIAGE_DURATION:  f64 = 5.0;       // nurse takes 5 min per patient
const MEAN_TREATMENT:   f64 = 20.0;      // mean treatment time in minutes
const CRITICAL_PROB:    f64 = 0.3;       // 30 % of patients are critical

const BLOOD_CAPACITY:   f64 = 100.0;     // units of blood in the bank
const BLOOD_INITIAL:    f64 = 60.0;      // starting level
const BLOOD_RESTOCK:    f64 = 20.0;      // units added every restock
const RESTOCK_INTERVAL: f64 = 60.0;     // minutes between restocks

const BLOOD_CRITICAL:   f64 = 10.0;     // units used by a critical patient
const BLOOD_STANDARD:   f64 = 2.0;      // units used by a standard patient

// ---------------------------------------------------------------------------
// Shared simulation context
// ---------------------------------------------------------------------------

type Log = Rc<RefCell<Vec<String>>>;

/// Personal eviction triggers for patients currently occupying a bed.
/// BTreeMap keeps insertion order by patient_id so we always evict the
/// longest-admitted patient first (lowest id = earliest arrival).
type EvictionMap = Rc<RefCell<BTreeMap<u32, EventTrigger>>>;

struct Stats {
    critical_treated:  u32,
    standard_treated:  u32,
    early_discharged:  u32,
    blood_bank_waits:  u32,
    total_nurse_wait:  f64,
    total_bed_wait:    f64,
    total_blood_wait:  f64,
}

#[derive(Clone)]
struct HospitalCtx {
    nurse:        PriorityResource,
    beds:         Resource,
    blood_bank:   Container,
    eviction_map: EvictionMap,
    log:          Log,
    stats:        Rc<RefCell<Stats>>,
}

pub struct SimResult {
    pub seed:             u64,
    pub critical_treated: u32,
    pub standard_treated: u32,
    pub early_discharged: u32,
    pub blood_bank_waits: u32,
    pub mean_nurse_wait:  f64,
    pub mean_bed_wait:    f64,
    pub mean_blood_wait:  f64,
}

// ---------------------------------------------------------------------------
// Simulation processes
// ---------------------------------------------------------------------------

/// Periodic blood bank replenishment.
async fn blood_bank_restock(env: EnvHandle, ctx: HospitalCtx) {
    loop {
        env.timeout(RESTOCK_INTERVAL).await;
        if env.now() > SIM_DURATION { break; }
        let before = ctx.blood_bank.level();
        ctx.blood_bank.put(BLOOD_RESTOCK).await;
        ctx.log.borrow_mut().push(format!(
            "[t={:5.1}] Blood bank restocked +{:.0}  (level: {:.0}/{:.0})",
            env.now(), BLOOD_RESTOCK, ctx.blood_bank.level(), ctx.blood_bank.capacity(),
        ));
        let _ = before; // suppress unused warning
    }
}

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

/// A single patient: triage nurse → blood draw → bed → treatment (possibly early-discharged).
async fn patient(
    env: EnvHandle,
    id: u32,
    triage: u32,
    treatment_duration: f64,
    ctx: HospitalCtx,
) {
    let label       = if triage == 0 { "CRITICAL" } else { "standard" };
    let blood_units = if triage == 0 { BLOOD_CRITICAL } else { BLOOD_STANDARD };
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

    // --- Blood draw (Container) ---
    let blood_wait_start = env.now();
    ctx.blood_bank.get(blood_units).await;
    let blood_wait = env.now() - blood_wait_start;
    if blood_wait > 0.0 {
        ctx.stats.borrow_mut().blood_bank_waits += 1;
        ctx.log.borrow_mut().push(format!(
            "[t={:5.1}] Patient {:2} blood draw done (waited {:.1} min, level: {:.0}/{:.0})",
            env.now(), id, blood_wait, ctx.blood_bank.level(), ctx.blood_bank.capacity(),
        ));
    }

    // --- Critical patients: evict the longest-admitted patient if beds are full ---
    if triage == 0 && ctx.beds.in_use() >= ctx.beds.capacity() {
        // BTreeMap iterates in ascending key (patient_id) order — lowest id first.
        let victim_id = ctx.eviction_map.borrow().keys().next().copied();
        if let Some(vid) = victim_id {
            if let Some(trigger) = ctx.eviction_map.borrow_mut().remove(&vid) {
                ctx.log.borrow_mut().push(format!(
                    "[t={:5.1}] Patient {:2} [CRITICAL] triggers early discharge of patient {:2}",
                    env.now(), id, vid,
                ));
                trigger.fire();
            }
        }
    }

    // --- Wait for a bed ---
    let bed_wait_start = env.now();
    let _bed = ctx.beds.request().await;
    let bed_wait = env.now() - bed_wait_start;
    let admitted_at = env.now();
    ctx.log.borrow_mut().push(format!(
        "[t={:5.1}] Patient {:2} admitted        (beds: {}/{})",
        env.now(), id, ctx.beds.in_use(), ctx.beds.capacity(),
    ));

    // --- Register personal eviction signal, then race treatment vs. eviction ---
    let (my_trigger, my_signal) = env.event();
    ctx.eviction_map.borrow_mut().insert(id, my_trigger);

    any_of![env.timeout(treatment_duration), my_signal].await;

    // Detect which branch won: eviction fires before the full treatment elapses.
    let was_early = env.now() < admitted_at + treatment_duration;

    // Clean up our eviction entry (no-op if already evicted and removed).
    ctx.eviction_map.borrow_mut().remove(&id);

    if was_early {
        ctx.log.borrow_mut().push(format!(
            "[t={:5.1}] Patient {:2} EARLY discharge   (beds: {}/{})",
            env.now(), id, ctx.beds.in_use() - 1, ctx.beds.capacity(),
        ));
    } else {
        ctx.log.borrow_mut().push(format!(
            "[t={:5.1}] Patient {:2} discharged        (beds: {}/{})",
            env.now(), id, ctx.beds.in_use() - 1, ctx.beds.capacity(),
        ));
    }

    let mut s = ctx.stats.borrow_mut();
    if triage == 0 { s.critical_treated += 1; } else { s.standard_treated += 1; }
    if was_early   { s.early_discharged += 1; }
    s.total_nurse_wait += nurse_wait;
    s.total_bed_wait   += bed_wait;
    s.total_blood_wait += blood_wait;
}

// ---------------------------------------------------------------------------
// Single-run entry point
// ---------------------------------------------------------------------------

fn run_simulation(seed: u64) -> SimResult {
    let mut env = SimEnv::with_seed(seed);
    let h = env.handle();

    let ctx = HospitalCtx {
        nurse:        PriorityResource::new(1),
        beds:         Resource::new(3),
        blood_bank:   Container::new(BLOOD_CAPACITY, BLOOD_INITIAL),
        eviction_map: Rc::new(RefCell::new(BTreeMap::new())),
        log:          Rc::new(RefCell::new(Vec::new())),
        stats:        Rc::new(RefCell::new(Stats {
            critical_treated: 0,
            standard_treated: 0,
            early_discharged: 0,
            blood_bank_waits: 0,
            total_nurse_wait: 0.0,
            total_bed_wait:   0.0,
            total_blood_wait: 0.0,
        })),
    };

    env.spawn(blood_bank_restock(h.clone(), ctx.clone()));
    env.spawn(arrivals(h, ctx.clone()));
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
        blood_bank_waits: s.blood_bank_waits,
        mean_nurse_wait: if total > 0 { s.total_nurse_wait / total as f64 } else { 0.0 },
        mean_bed_wait:   if total > 0 { s.total_bed_wait   / total as f64 } else { 0.0 },
        mean_blood_wait: if total > 0 { s.total_blood_wait / total as f64 } else { 0.0 },
    }
}

// ---------------------------------------------------------------------------
// Main: run 10 seeds in parallel, print summary table
// ---------------------------------------------------------------------------

fn main() {
    let results = simu::monte_carlo::run(0..10, run_simulation);

    println!(
        "{:>6}  {:>8}  {:>8}  {:>7}  {:>11}  {:>17}  {:>14}  {:>15}",
        "Seed", "Critical", "Standard", "Early", "Blood waits",
        "Nurse wait (mean)", "Bed wait (mean)", "Blood wait (mean)",
    );
    println!("{}", "-".repeat(100));

    for r in &results {
        println!(
            "{:>6}  {:>8}  {:>8}  {:>7}  {:>11}  {:>17.1}  {:>14.1}  {:>15.1}",
            r.seed, r.critical_treated, r.standard_treated, r.early_discharged,
            r.blood_bank_waits,
            r.mean_nurse_wait, r.mean_bed_wait, r.mean_blood_wait,
        );
    }

    let n = results.len() as f64;
    let mean_crit  = results.iter().map(|r| r.critical_treated as f64).sum::<f64>() / n;
    let mean_std   = results.iter().map(|r| r.standard_treated as f64).sum::<f64>() / n;
    let mean_early = results.iter().map(|r| r.early_discharged as f64).sum::<f64>() / n;
    let mean_bw    = results.iter().map(|r| r.blood_bank_waits as f64).sum::<f64>() / n;
    let mean_nurse = results.iter().map(|r| r.mean_nurse_wait).sum::<f64>() / n;
    let mean_bed   = results.iter().map(|r| r.mean_bed_wait).sum::<f64>()   / n;
    let mean_blood = results.iter().map(|r| r.mean_blood_wait).sum::<f64>() / n;

    println!("{}", "-".repeat(100));
    println!(
        "{:>6}  {:>8.1}  {:>8.1}  {:>7.1}  {:>11.1}  {:>17.1}  {:>14.1}  {:>15.1}",
        "mean", mean_crit, mean_std, mean_early, mean_bw,
        mean_nurse, mean_bed, mean_blood,
    );

    println!("\nPer-run logs written to run_00.log … run_09.log");
}

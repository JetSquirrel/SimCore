use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::File;
use std::future::Future;
use std::io::Write;
use std::pin::Pin;
use std::rc::Rc;

use rand::Rng;
use rand_distr::{Exp, Normal};
use simu::{any_of, AllOf, Container, EnvHandle, EventTrigger, PriorityResource, Resource, SimEnv};

// ---------------------------------------------------------------------------
// Brewery simulation — a craft brewery producing beer in batches.
//
// Demonstrates every public primitive of `simu`:
//
//  • Resource          : mash tuns, kettles, fermenters (the bio-reactors),
//                        conditioning tanks
//  • PriorityResource  : bottling line (premium batches first) and CIP crew
//                        (contaminated batches preempt routine cleanups)
//  • Container         : hot-water buffer, yeast-slurry pool, CO₂ recovery
//                        tank, bulk-beer storage
//  • EventTrigger      : per-batch contamination signal
//  • any_of!           : fermentation timer races against contamination
//  • ProcessHandle     : arrivals collects each batch's handle; AllOf joins
//                        them at end-of-week so we can log the simulated
//                        time the line is fully drained
//  • monte_carlo::run  : ten independent seeds in parallel
//
// The QA inspector picks the longest-running fermentation (lowest batch id
// in a BTreeMap) and fires its contamination trigger. The fermenting batch
// resolves its any_of! early, releases the bio-reactor, and skips straight
// to an urgent CIP that preempts any routine cleanup behind it.
// ---------------------------------------------------------------------------

const SIM_DURATION:        f64 = 168.0;      // one week, in hours
const ARRIVAL_RATE:        f64 = 1.0 / 12.0; // one batch every ~12 h
const PREMIUM_PROB:        f64 = 0.25;       // 25 % of orders are premium

const MASH_DURATION:       f64 = 2.0;
const BOIL_DURATION:       f64 = 1.5;
const FERMENT_MEAN:        f64 = 60.0;       // ~2.5 days (fast ale)
const FERMENT_STD:         f64 = 6.0;
const CONDITION_DURATION:  f64 = 12.0;
const BOTTLE_DURATION:     f64 = 6.0;        // slow enough that premium priority matters
const CIP_DURATION:        f64 = 1.0;
const URGENT_CIP_DURATION: f64 = 2.5;        // contaminated batches need a thorough clean

const HOT_WATER_CAPACITY:  f64 = 2000.0;
const HOT_WATER_INITIAL:   f64 = 1500.0;
const HOT_WATER_PER_MASH:  f64 = 500.0;
const HOT_WATER_RESTOCK:   f64 = 400.0;
const HOT_WATER_INTERVAL:  f64 = 4.0;

const YEAST_CAPACITY:      f64 = 100.0;
const YEAST_INITIAL:       f64 = 30.0;
const YEAST_PER_BATCH:     f64 = 5.0;
const YEAST_PROPAGATE:     f64 = 8.0;        // tight on purpose — yeast occasionally blocks
const YEAST_INTERVAL:      f64 = 24.0;

const CO2_CAPACITY:        f64 = 10_000.0;
const CO2_PER_BOIL:        f64 = 40.0;

const BEER_BUFFER_CAP:     f64 = 5_000.0;
const BEER_PER_BATCH:      f64 = 800.0;     // L produced from one full fermenter

const INSPECTION_RATE:     f64 = 1.0 / 30.0; // ~ one QA inspection every 30 h
const CONTAMINATION_PROB:  f64 = 0.40;       // probability an inspection finds contamination

// ---------------------------------------------------------------------------
// Shared simulation context
// ---------------------------------------------------------------------------

type Log = Rc<RefCell<Vec<String>>>;

/// Per-batch contamination triggers for batches currently fermenting.
/// BTreeMap by batch_id keeps insertion order — `keys().next()` always
/// yields the oldest in-flight fermentation, which is what QA picks.
type ContaminationMap = Rc<RefCell<BTreeMap<u32, EventTrigger>>>;

#[derive(Default)]
struct Stats {
    batches_arrived:      u32,
    premium_arrived:      u32,
    completed_premium:    u32,
    completed_standard:   u32,
    contaminated:         u32,
    litres_bottled:       f64,
    yeast_waits:          u32,
    total_ferment_wait:   f64,
    total_bottling_wait:  f64,
    total_yeast_wait:     f64,
    line_cleared_at:      f64,
}

#[derive(Clone)]
struct BreweryCtx {
    mash_tuns:           Resource,
    kettles:             Resource,
    fermenters:          Resource,
    conditioning_tanks:  Resource,
    bottling_line:       PriorityResource,
    cip_crew:            PriorityResource,
    hot_water:           Container,
    yeast_slurry:        Container,
    co2_recovery:        Container,
    bulk_beer:           Container,
    contamination_map:   ContaminationMap,
    log:                 Log,
    stats:               Rc<RefCell<Stats>>,
}

pub struct SimResult {
    pub seed:               u64,
    pub batches_arrived:    u32,
    pub completed_premium:  u32,
    pub completed_standard: u32,
    pub contaminated:       u32,
    pub litres_bottled:     f64,
    pub yeast_waits:        u32,
    pub mean_ferment_wait:  f64,
    pub mean_bottling_wait: f64,
    pub mean_yeast_wait:    f64,
    pub line_cleared_at:    f64,
}

// ---------------------------------------------------------------------------
// Background processes
// ---------------------------------------------------------------------------

/// Periodic hot-water replenishment.
async fn hot_water_replenishment(env: EnvHandle, ctx: BreweryCtx) {
    loop {
        env.timeout(HOT_WATER_INTERVAL).await;
        if env.now() > SIM_DURATION { break; }
        ctx.hot_water.put(HOT_WATER_RESTOCK).await;
        ctx.log.borrow_mut().push(format!(
            "[t={:6.1}] Hot water  +{:>4.0}      (level: {:>5.0}/{:>5.0})",
            env.now(), HOT_WATER_RESTOCK,
            ctx.hot_water.level(), ctx.hot_water.capacity(),
        ));
    }
}

/// Periodic yeast propagation.
async fn yeast_propagation(env: EnvHandle, ctx: BreweryCtx) {
    loop {
        env.timeout(YEAST_INTERVAL).await;
        if env.now() > SIM_DURATION { break; }
        ctx.yeast_slurry.put(YEAST_PROPAGATE).await;
        ctx.log.borrow_mut().push(format!(
            "[t={:6.1}] Yeast      +{:>4.0}      (level: {:>5.0}/{:>5.0})",
            env.now(), YEAST_PROPAGATE,
            ctx.yeast_slurry.level(), ctx.yeast_slurry.capacity(),
        ));
    }
}

/// QA inspector: at random intervals, finds contamination in the
/// longest-running fermentation (lowest batch id) with probability
/// CONTAMINATION_PROB. Fires that batch's contamination signal — the
/// fermenter resolves its `any_of!` early and skips straight to urgent CIP.
async fn qa_inspector(env: EnvHandle, ctx: BreweryCtx) {
    let inspect_dist = Exp::new(INSPECTION_RATE).unwrap();
    loop {
        let dt = env.rng().sample(inspect_dist);
        env.timeout(dt).await;
        if env.now() > SIM_DURATION { break; }

        let hit = env.rng().gen::<f64>() < CONTAMINATION_PROB;
        if !hit { continue; }

        let victim_id = ctx.contamination_map.borrow().keys().next().copied();
        let Some(vid) = victim_id else { continue };

        if let Some(trigger) = ctx.contamination_map.borrow_mut().remove(&vid) {
            ctx.log.borrow_mut().push(format!(
                "[t={:6.1}] QA inspector flags batch {:>3} as CONTAMINATED",
                env.now(), vid,
            ));
            trigger.fire();
        }
    }
}

// ---------------------------------------------------------------------------
// Per-batch lifecycle: mash → boil → ferment → (condition → bottle) → CIP
// ---------------------------------------------------------------------------

async fn batch_lifecycle(env: EnvHandle, id: u32, premium: bool, ctx: BreweryCtx) {
    let label = if premium { "PREMIUM " } else { "standard" };
    ctx.log.borrow_mut().push(format!(
        "[t={:6.1}] Batch {:>3} arrives                     [{}]",
        env.now(), id, label,
    ));

    // --- 1. Mashing ------------------------------------------------------
    let _mash = ctx.mash_tuns.request().await;
    ctx.hot_water.get(HOT_WATER_PER_MASH).await;
    ctx.log.borrow_mut().push(format!(
        "[t={:6.1}] Batch {:>3} mashing                     (water: {:>5.0}/{:>5.0})",
        env.now(), id, ctx.hot_water.level(), ctx.hot_water.capacity(),
    ));
    env.timeout(MASH_DURATION).await;
    drop(_mash);

    // --- 2. Boiling + hopping --------------------------------------------
    let _kettle = ctx.kettles.request().await;
    ctx.log.borrow_mut().push(format!(
        "[t={:6.1}] Batch {:>3} boiling",
        env.now(), id,
    ));
    env.timeout(BOIL_DURATION).await;
    ctx.co2_recovery.put(CO2_PER_BOIL).await;
    drop(_kettle);

    // --- 3. Fermentation (the bio-reactor) -------------------------------
    let yeast_wait_start = env.now();
    ctx.yeast_slurry.get(YEAST_PER_BATCH).await;
    let yeast_wait = env.now() - yeast_wait_start;
    if yeast_wait > 0.0 {
        ctx.stats.borrow_mut().yeast_waits += 1;
    }

    let ferment_wait_start = env.now();
    let _fermenter = ctx.fermenters.request().await;
    let ferment_wait = env.now() - ferment_wait_start;

    let ferment_dist = Normal::new(FERMENT_MEAN, FERMENT_STD).unwrap();
    let ferment_duration = env.rng().sample(ferment_dist).max(1.0);
    let ferment_start = env.now();

    ctx.log.borrow_mut().push(format!(
        "[t={:6.1}] Batch {:>3} fermenting                  (yeast wait {:>4.1} h, ferm wait {:>4.1} h, planned {:>5.1} h)",
        env.now(), id, yeast_wait, ferment_wait, ferment_duration,
    ));

    let (my_trigger, my_signal) = env.event();
    ctx.contamination_map.borrow_mut().insert(id, my_trigger);

    any_of![env.timeout(ferment_duration), my_signal].await;

    // The contamination branch fires before the natural timer would have
    // elapsed. Comparing now() against the planned deadline tells us which.
    let was_contaminated = env.now() < ferment_start + ferment_duration;
    ctx.contamination_map.borrow_mut().remove(&id);
    drop(_fermenter);

    {
        let mut s = ctx.stats.borrow_mut();
        s.total_ferment_wait += ferment_wait;
        s.total_yeast_wait   += yeast_wait;
    }

    if was_contaminated {
        ctx.log.borrow_mut().push(format!(
            "[t={:6.1}] Batch {:>3} ABORTED — urgent CIP",
            env.now(), id,
        ));
        // Urgent CIP — priority 0 preempts any routine cleanup behind it.
        let _crew = ctx.cip_crew.request(0).await;
        env.timeout(URGENT_CIP_DURATION).await;
        drop(_crew);
        ctx.stats.borrow_mut().contaminated += 1;
        ctx.log.borrow_mut().push(format!(
            "[t={:6.1}] Batch {:>3} urgent CIP done",
            env.now(), id,
        ));
        return;
    }

    // --- 4. Conditioning -------------------------------------------------
    let _tank = ctx.conditioning_tanks.request().await;
    ctx.log.borrow_mut().push(format!(
        "[t={:6.1}] Batch {:>3} conditioning",
        env.now(), id,
    ));
    env.timeout(CONDITION_DURATION).await;
    ctx.bulk_beer.put(BEER_PER_BATCH).await;
    drop(_tank);

    // --- 5. Bottling (premium = priority 0, standard = priority 1) -------
    let bottling_wait_start = env.now();
    let bottle_priority = if premium { 0_u32 } else { 1_u32 };
    let _line = ctx.bottling_line.request(bottle_priority).await;
    let bottling_wait = env.now() - bottling_wait_start;
    ctx.bulk_beer.get(BEER_PER_BATCH).await;
    env.timeout(BOTTLE_DURATION).await;
    drop(_line);

    ctx.log.borrow_mut().push(format!(
        "[t={:6.1}] Batch {:>3} bottled                     [{}]  (bottling wait {:>4.1} h, {:>4.0} L)",
        env.now(), id, label, bottling_wait, BEER_PER_BATCH,
    ));

    // --- 6. Routine CIP (priority 1) -------------------------------------
    let _crew = ctx.cip_crew.request(1).await;
    env.timeout(CIP_DURATION).await;
    drop(_crew);

    let mut s = ctx.stats.borrow_mut();
    if premium { s.completed_premium += 1; } else { s.completed_standard += 1; }
    s.litres_bottled      += BEER_PER_BATCH;
    s.total_bottling_wait += bottling_wait;
}

// ---------------------------------------------------------------------------
// Arrivals: spawns one batch per Poisson tick, joins them all at the end.
// ---------------------------------------------------------------------------

async fn arrivals(env: EnvHandle, ctx: BreweryCtx) {
    let arrival_dist = Exp::new(ARRIVAL_RATE).unwrap();
    let mut batch_id = 1_u32;
    let mut batch_futs: Vec<Pin<Box<dyn Future<Output = ()>>>> = Vec::new();

    loop {
        let inter_arrival = env.rng().sample(arrival_dist);
        env.timeout(inter_arrival).await;

        if env.now() > SIM_DURATION {
            ctx.log.borrow_mut().push(format!(
                "[t={:6.1}] Order book closed ({} orders accepted)",
                env.now(), batch_id - 1,
            ));
            break;
        }

        let premium = env.rng().gen::<f64>() < PREMIUM_PROB;
        {
            let mut s = ctx.stats.borrow_mut();
            s.batches_arrived += 1;
            if premium { s.premium_arrived += 1; }
        }

        let handle = env.spawn(
            batch_lifecycle(env.clone(), batch_id, premium, ctx.clone()),
        );
        batch_futs.push(Box::pin(handle.discard()));
        batch_id += 1;
    }

    // Wait for every batch to finish its CIP, with a hard deadline so the
    // simulation always terminates even if a batch is stuck waiting on a
    // depleted yeast pool after propagation has shut down.
    if !batch_futs.is_empty() {
        let all_batches = AllOf::new(batch_futs);
        any_of![all_batches, env.timeout(SIM_DURATION * 10.0)].await;
    }
    ctx.stats.borrow_mut().line_cleared_at = env.now();
    ctx.log.borrow_mut().push(format!(
        "[t={:6.1}] Line clear marker", env.now(),
    ));
}

// ---------------------------------------------------------------------------
// Single-run entry point
// ---------------------------------------------------------------------------

fn run_simulation(seed: u64) -> SimResult {
    let mut env = SimEnv::with_seed(seed);
    let h = env.handle();

    let ctx = BreweryCtx {
        mash_tuns:          Resource::new(2),
        kettles:            Resource::new(2),
        fermenters:         Resource::new(5),
        conditioning_tanks: Resource::new(4),
        bottling_line:      PriorityResource::new(1),
        cip_crew:           PriorityResource::new(1),
        hot_water:          Container::new(HOT_WATER_CAPACITY, HOT_WATER_INITIAL),
        yeast_slurry:       Container::new(YEAST_CAPACITY, YEAST_INITIAL),
        co2_recovery:       Container::empty(CO2_CAPACITY),
        bulk_beer:          Container::empty(BEER_BUFFER_CAP),
        contamination_map:  Rc::new(RefCell::new(BTreeMap::new())),
        log:                Rc::new(RefCell::new(Vec::new())),
        stats:              Rc::new(RefCell::new(Stats::default())),
    };

    env.spawn(hot_water_replenishment(h.clone(), ctx.clone()));
    env.spawn(yeast_propagation(h.clone(), ctx.clone()));
    env.spawn(qa_inspector(h.clone(), ctx.clone()));
    env.spawn(arrivals(h, ctx.clone()));
    env.run();

    let path = format!("brewery_run_{:02}.log", seed);
    let mut file = File::create(&path).expect("could not create log file");
    for line in ctx.log.borrow().iter() {
        writeln!(file, "{}", line).unwrap();
    }

    let s = ctx.stats.borrow();
    let completed = s.completed_premium + s.completed_standard;
    SimResult {
        seed,
        batches_arrived:    s.batches_arrived,
        completed_premium:  s.completed_premium,
        completed_standard: s.completed_standard,
        contaminated:       s.contaminated,
        litres_bottled:     s.litres_bottled,
        yeast_waits:        s.yeast_waits,
        mean_ferment_wait:  if s.batches_arrived > 0 { s.total_ferment_wait / s.batches_arrived as f64 } else { 0.0 },
        mean_bottling_wait: if completed > 0          { s.total_bottling_wait / completed as f64        } else { 0.0 },
        mean_yeast_wait:    if s.batches_arrived > 0 { s.total_yeast_wait   / s.batches_arrived as f64 } else { 0.0 },
        line_cleared_at:    s.line_cleared_at,
    }
}

// ---------------------------------------------------------------------------
// Main: run 10 seeds in parallel, print summary table
// ---------------------------------------------------------------------------

fn main() {
    let results = simu::monte_carlo::run(0..10, run_simulation);

    println!(
        "{:>4}  {:>8}  {:>7}  {:>8}  {:>5}  {:>8}  {:>6}  {:>11}  {:>12}  {:>11}  {:>9}",
        "Seed", "Arrivals", "Premium", "Standard", "Cont.",
        "Litres", "Yeast", "Ferment(h)", "Bottling(h)", "Yeast(h)", "Cleared",
    );
    println!("{}", "-".repeat(112));

    for r in &results {
        println!(
            "{:>4}  {:>8}  {:>7}  {:>8}  {:>5}  {:>8.0}  {:>6}  {:>11.1}  {:>12.1}  {:>11.2}  {:>9.1}",
            r.seed, r.batches_arrived,
            r.completed_premium, r.completed_standard, r.contaminated,
            r.litres_bottled, r.yeast_waits,
            r.mean_ferment_wait, r.mean_bottling_wait, r.mean_yeast_wait,
            r.line_cleared_at,
        );
    }

    let n = results.len() as f64;
    let mean = |f: fn(&SimResult) -> f64| results.iter().map(f).sum::<f64>() / n;

    println!("{}", "-".repeat(112));
    println!(
        "{:>4}  {:>8.1}  {:>7.1}  {:>8.1}  {:>5.1}  {:>8.0}  {:>6.1}  {:>11.1}  {:>12.1}  {:>11.2}  {:>9.1}",
        "mean",
        mean(|r| r.batches_arrived    as f64),
        mean(|r| r.completed_premium  as f64),
        mean(|r| r.completed_standard as f64),
        mean(|r| r.contaminated       as f64),
        mean(|r| r.litres_bottled),
        mean(|r| r.yeast_waits        as f64),
        mean(|r| r.mean_ferment_wait),
        mean(|r| r.mean_bottling_wait),
        mean(|r| r.mean_yeast_wait),
        mean(|r| r.line_cleared_at),
    );

    println!("\nPer-run logs written to brewery_run_00.log … brewery_run_09.log");
}

use std::cell::RefCell;
use std::fs::File;
use std::future::Future;
use std::io::Write;
use std::path::PathBuf;
use std::pin::Pin;
use std::rc::Rc;

use rand::Rng;
use rand_distr::{Exp, Normal};
use simu::{any_of, AllOf, Container, EnvHandle, PreemptiveResource, PriorityResource, Resource, SimEnv};

// ---------------------------------------------------------------------------
// Warehouse / distribution-center simulation — goods flow in on trucks and out
// on customer orders, sharing one small fleet of forklifts between the two.
//
// Demonstrates every public primitive of `simu`, with a preemptible forklift
// fleet as the centerpiece (the first example to exercise `PreemptiveResource`):
//
//  • PreemptiveResource : forklift fleet — routine putaway (priority 1) is
//                         evicted by truck-side unload / load (priority 0); the
//                         bumped driver parks the pallet and finishes it later
//  • Resource           : dock doors and packing stations (FIFO, capacity-bound)
//  • PriorityResource   : pickers — expedite orders jump the queue ahead of
//                         standard ones, but never evict an in-progress pick
//  • Container          : on-hand inventory in cases — inbound putaway `put`s
//                         stock, outbound picking `get`s it; a stockout suspends
//                         a picker until the next putaway lands
//  • any_of! + preempted: putaway races its work against `guard.preempted()`
//  • ProcessHandle+AllOf: each arrival stream collects its spawned handles and
//                         joins them at end-of-day to log when the floor clears
//  • monte_carlo::run   : ten independent seeds in parallel
//
// The two streams meet at the single shared inventory pool: you cannot ship
// what has not yet been put away, so forklift contention on the inbound side
// can starve the outbound pick face — the most interesting emergent behavior.
// ---------------------------------------------------------------------------

const SIM_DURATION: f64 = 600.0; // 10-hour day, in minutes

const TRUCK_ARRIVAL_RATE: f64 = 1.0 / 45.0; // Poisson, ~1 inbound truck every 45 min
const ORDER_ARRIVAL_RATE: f64 = 1.0 / 12.0; // Poisson, ~1 order every 12 min
const EXPEDITE_PROB: f64 = 0.20; // 20 % of orders are expedite / "hot"

// Inbound (receiving)
const UNLOAD_DURATION: f64 = 30.0; // palletized trailer, forklift @ PRIO_TRUCK
const QC_DURATION: f64 = 10.0; // receiving check vs. ASN (no resource — a timeout)
const PUTAWAY_DURATION: f64 = 20.0; // forklift @ PRIO_PUTAWAY — the preemptible task
const CASES_PER_DELIVERY: f64 = 180.0; // cases added to stock once putaway completes

// Outbound (fulfillment)
const PICK_MEAN: f64 = 8.0; // Exp(1/8) travel + pick time
const PACK_DURATION: f64 = 5.0; // packing-station time
const LOAD_DURATION: f64 = 6.0; // forklift @ PRIO_TRUCK — load onto trailer
const ORDER_CASES_MEAN: f64 = 40.0; // Normal(40, 12), clamped >= 1
const ORDER_CASES_STD: f64 = 12.0;

// Inventory (single abstracted stock pool, in cases)
const STOCK_CAPACITY: f64 = 2000.0;
const STOCK_INITIAL: f64 = 800.0; // safety stock — enough to ride out a thin receiving day

// Forklift priorities (PreemptiveResource — lower = higher priority)
const PRIO_TRUCK: u32 = 0; // unload / load: a truck is at the dock, clock running
const PRIO_PUTAWAY: u32 = 1; // routine putaway: parkable, therefore preemptible

// Picker priorities (PriorityResource — non-preemptive)
const PICK_EXPEDITE: u32 = 0;
const PICK_STANDARD: u32 = 1;

// ---------------------------------------------------------------------------
// Shared simulation context
// ---------------------------------------------------------------------------

type Log = Rc<RefCell<Vec<String>>>;

#[derive(Default)]
struct Stats {
    trucks: u32,
    orders: u32,
    completed_expedite: u32,
    completed_standard: u32,
    putaway_preemptions: u32,
    stockouts: u32,
    total_dock_wait: f64,
    dock_requests: u32,
    total_fork_wait: f64,
    fork_requests: u32,
    total_pick_wait: f64,
    pick_requests: u32,
    day_cleared_at: f64,
}

#[derive(Clone)]
struct WarehouseCtx {
    dock_doors: Resource,
    forklifts: PreemptiveResource,
    pickers: PriorityResource,
    packing: Resource,
    inventory: Container,
    log: Log,
    stats: Rc<RefCell<Stats>>,
}

pub struct SimResult {
    pub seed: u64,
    pub trucks: u32,
    pub orders: u32,
    pub completed_expedite: u32,
    pub completed_standard: u32,
    pub putaway_preemptions: u32,
    pub stockouts: u32,
    pub mean_dock_wait: f64,
    pub mean_fork_wait: f64,
    pub mean_pick_wait: f64,
    pub day_cleared_at: f64,
}

// ---------------------------------------------------------------------------
// Inbound truck: dock -> unload -> QC -> putaway (preemptible) -> stock
// ---------------------------------------------------------------------------

async fn inbound_truck(env: EnvHandle, id: u32, ctx: WarehouseCtx) {
    ctx.log.borrow_mut().push(format!(
        "[t={:6.1}] Truck {:>3} arrives at the yard",
        env.now(),
        id,
    ));

    // --- 1. Dock door + unload (forklift @ PRIO_TRUCK — never preempted) ---
    let dock_t0 = env.now();
    let dock = ctx.dock_doors.request().await;
    let dock_wait = env.now() - dock_t0;

    let fork_t0 = env.now();
    let fork = ctx.forklifts.request(PRIO_TRUCK).await;
    let fork_wait = env.now() - fork_t0;

    ctx.log.borrow_mut().push(format!(
        "[t={:6.1}] Truck {:>3} unloading                  (dock wait {:>5.1}, fork wait {:>5.1})",
        env.now(),
        id,
        dock_wait,
        fork_wait,
    ));
    env.timeout(UNLOAD_DURATION).await;
    // Free the scarce dock door (and the forklift) the moment the pallets are
    // on the floor — QC and putaway do not need a door.
    drop(fork);
    drop(dock);

    {
        let mut s = ctx.stats.borrow_mut();
        s.total_dock_wait += dock_wait;
        s.dock_requests += 1;
        s.total_fork_wait += fork_wait;
        s.fork_requests += 1;
    }

    // --- 2. Receiving check / QC (no resource — a plain timeout) ---
    env.timeout(QC_DURATION).await;

    // --- 3. Putaway: the only preemptible forklift task ---
    // A half-finished putaway is safely parked; race the work against
    // preemption and loop to finish the remainder if a truck-side job bumps us.
    let mut remaining = PUTAWAY_DURATION;
    loop {
        let pf_t0 = env.now();
        let pfork = ctx.forklifts.request(PRIO_PUTAWAY).await; // priority 1
        {
            let mut s = ctx.stats.borrow_mut();
            s.total_fork_wait += env.now() - pf_t0;
            s.fork_requests += 1;
        }

        let start = env.now();
        any_of![env.timeout(remaining), pfork.preempted()].await;
        remaining -= env.now() - start;

        if pfork.is_preempted() {
            ctx.stats.borrow_mut().putaway_preemptions += 1;
            ctx.log.borrow_mut().push(format!(
                "[t={:6.1}] Truck {:>3} putaway PREEMPTED          (pallet parked, {:>4.1} left)",
                env.now(),
                id,
                remaining,
            ));
            // Pallet parked in staging; loop to reacquire a forklift later.
            // (Dropping an already-preempted guard is a no-op — the unit is gone.)
            continue;
        }
        break; // finished uninterrupted; `pfork` drops here, freeing the unit
    }

    // Stock becomes available only now — after putaway has fully completed.
    ctx.inventory.put(CASES_PER_DELIVERY).await;
    ctx.log.borrow_mut().push(format!(
        "[t={:6.1}] Truck {:>3} putaway done  +{:>4.0} cases  (stock {:>5.0}/{:>5.0})",
        env.now(),
        id,
        CASES_PER_DELIVERY,
        ctx.inventory.level(),
        ctx.inventory.capacity(),
    ));
}

// ---------------------------------------------------------------------------
// Outbound order: pick -> inventory.get -> pack -> load -> ship
// ---------------------------------------------------------------------------

async fn order_fulfillment(env: EnvHandle, id: u32, expedite: bool, ctx: WarehouseCtx) {
    let label = if expedite { "EXPEDITE" } else { "standard" };

    // Sample the order size before any await (the RNG guard is !Send and
    // cannot be held across an await point).
    let cases = {
        let dist = Normal::new(ORDER_CASES_MEAN, ORDER_CASES_STD).unwrap();
        env.rng().sample(dist).max(1.0)
    };
    ctx.log.borrow_mut().push(format!(
        "[t={:6.1}] Order {:>3} received   {:>4.0} cases    [{}]",
        env.now(),
        id,
        cases,
        label,
    ));

    // --- 1. Picking (priority-scheduled, non-preemptive) ---
    let pick_prio = if expedite { PICK_EXPEDITE } else { PICK_STANDARD };
    let pick_t0 = env.now();
    let picker = ctx.pickers.request(pick_prio).await;
    let pick_wait = env.now() - pick_t0;

    let pick_dur = {
        let dist = Exp::new(1.0 / PICK_MEAN).unwrap();
        env.rng().sample(dist)
    };
    env.timeout(pick_dur).await;

    // The `inventory.get` sits *inside* the picker hold: a picker who reaches
    // an empty face waits there, still occupying its slot, until an inbound
    // putaway replenishes stock. This couples outbound to inbound.
    let stock_t0 = env.now();
    ctx.inventory.get(cases).await;
    if env.now() > stock_t0 {
        ctx.stats.borrow_mut().stockouts += 1;
        ctx.log.borrow_mut().push(format!(
            "[t={:6.1}] Order {:>3} picked through stockout   (picker stalled {:>5.1})",
            env.now(),
            id,
            env.now() - stock_t0,
        ));
    }
    drop(picker);

    {
        let mut s = ctx.stats.borrow_mut();
        s.total_pick_wait += pick_wait;
        s.pick_requests += 1;
    }

    // --- 2. Packing ---
    let pack = ctx.packing.request().await;
    env.timeout(PACK_DURATION).await;
    drop(pack);

    // --- 3. Loading onto the outbound trailer (dock + forklift @ PRIO_TRUCK) ---
    // Loading is urgent truck-side work: it will preempt a routine putaway.
    let dock_t0 = env.now();
    let dock = ctx.dock_doors.request().await;
    let dock_wait = env.now() - dock_t0;

    let fork_t0 = env.now();
    let fork = ctx.forklifts.request(PRIO_TRUCK).await;
    let fork_wait = env.now() - fork_t0;

    env.timeout(LOAD_DURATION).await;
    drop(fork);
    drop(dock);

    {
        let mut s = ctx.stats.borrow_mut();
        s.total_dock_wait += dock_wait;
        s.dock_requests += 1;
        s.total_fork_wait += fork_wait;
        s.fork_requests += 1;
        if expedite {
            s.completed_expedite += 1;
        } else {
            s.completed_standard += 1;
        }
    }
    ctx.log.borrow_mut().push(format!(
        "[t={:6.1}] Order {:>3} shipped                  [{}]  (pick wait {:>5.1})",
        env.now(),
        id,
        label,
        pick_wait,
    ));
}

// ---------------------------------------------------------------------------
// Arrival streams: each spawns one process per Poisson tick, then joins them
// all at end-of-day so we can record when the floor is fully drained.
// ---------------------------------------------------------------------------

async fn truck_arrivals(env: EnvHandle, ctx: WarehouseCtx) {
    let arrival_dist = Exp::new(TRUCK_ARRIVAL_RATE).unwrap();
    let mut truck_id = 1_u32;
    let mut truck_futs: Vec<Pin<Box<dyn Future<Output = ()>>>> = Vec::new();

    loop {
        let inter_arrival = env.rng().sample(arrival_dist);
        env.timeout(inter_arrival).await;

        if env.now() > SIM_DURATION {
            ctx.log.borrow_mut().push(format!(
                "[t={:6.1}] Inbound gate closed ({} trucks received)",
                env.now(),
                truck_id - 1,
            ));
            break;
        }

        ctx.stats.borrow_mut().trucks += 1;
        let handle = env.spawn(inbound_truck(env.clone(), truck_id, ctx.clone()));
        truck_futs.push(Box::pin(handle.discard()));
        truck_id += 1;
    }

    // Drain every truck that arrived, with a hard deadline so the run always
    // terminates even if a putaway is repeatedly preempted near end-of-day.
    if !truck_futs.is_empty() {
        let all = AllOf::new(truck_futs);
        any_of![all, env.timeout(SIM_DURATION * 10.0)].await;
    }
    let mut s = ctx.stats.borrow_mut();
    s.day_cleared_at = s.day_cleared_at.max(env.now());
}

async fn order_arrivals(env: EnvHandle, ctx: WarehouseCtx) {
    let arrival_dist = Exp::new(ORDER_ARRIVAL_RATE).unwrap();
    let mut order_id = 1_u32;
    let mut order_futs: Vec<Pin<Box<dyn Future<Output = ()>>>> = Vec::new();

    loop {
        let inter_arrival = env.rng().sample(arrival_dist);
        env.timeout(inter_arrival).await;

        if env.now() > SIM_DURATION {
            ctx.log.borrow_mut().push(format!(
                "[t={:6.1}] Order book closed ({} orders accepted)",
                env.now(),
                order_id - 1,
            ));
            break;
        }

        let expedite = env.rng().gen::<f64>() < EXPEDITE_PROB;
        ctx.stats.borrow_mut().orders += 1;
        let handle = env.spawn(order_fulfillment(env.clone(), order_id, expedite, ctx.clone()));
        order_futs.push(Box::pin(handle.discard()));
        order_id += 1;
    }

    if !order_futs.is_empty() {
        let all = AllOf::new(order_futs);
        any_of![all, env.timeout(SIM_DURATION * 10.0)].await;
    }
    let mut s = ctx.stats.borrow_mut();
    s.day_cleared_at = s.day_cleared_at.max(env.now());
}

// ---------------------------------------------------------------------------
// Single-run entry point
// ---------------------------------------------------------------------------

/// Directory for per-run logs: `<target>/sim-logs`, created if missing.
///
/// Writing under the build target directory keeps these artifacts out of the
/// source tree (and out of git — `/target` is gitignored). Honours
/// `CARGO_TARGET_DIR` when set, falling back to `target/` next to the crate.
fn log_dir() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
    let dir = target.join("sim-logs");
    std::fs::create_dir_all(&dir).expect("could not create log directory");
    dir
}

fn run_simulation(seed: u64) -> SimResult {
    let mut env = SimEnv::with_seed(seed);
    let h = env.handle();

    let ctx = WarehouseCtx {
        dock_doors: Resource::new(4),
        forklifts: PreemptiveResource::new(2),
        pickers: PriorityResource::new(4),
        packing: Resource::new(2),
        inventory: Container::new(STOCK_CAPACITY, STOCK_INITIAL),
        log: Rc::new(RefCell::new(Vec::new())),
        stats: Rc::new(RefCell::new(Stats::default())),
    };

    env.spawn(truck_arrivals(h.clone(), ctx.clone()));
    env.spawn(order_arrivals(h, ctx.clone()));
    env.run();

    let path = log_dir().join(format!("warehouse_run_{:02}.log", seed));
    let mut file = File::create(&path).expect("could not create log file");
    for line in ctx.log.borrow().iter() {
        writeln!(file, "{}", line).unwrap();
    }

    let s = ctx.stats.borrow();
    SimResult {
        seed,
        trucks: s.trucks,
        orders: s.orders,
        completed_expedite: s.completed_expedite,
        completed_standard: s.completed_standard,
        putaway_preemptions: s.putaway_preemptions,
        stockouts: s.stockouts,
        mean_dock_wait: if s.dock_requests > 0 {
            s.total_dock_wait / s.dock_requests as f64
        } else {
            0.0
        },
        mean_fork_wait: if s.fork_requests > 0 {
            s.total_fork_wait / s.fork_requests as f64
        } else {
            0.0
        },
        mean_pick_wait: if s.pick_requests > 0 {
            s.total_pick_wait / s.pick_requests as f64
        } else {
            0.0
        },
        day_cleared_at: s.day_cleared_at,
    }
}

// ---------------------------------------------------------------------------
// Main: run 10 seeds in parallel, print summary table
// ---------------------------------------------------------------------------

fn main() {
    let results = simu::monte_carlo::run(0..10, run_simulation);

    println!(
        "{:>4}  {:>7}  {:>7}  {:>8}  {:>8}  {:>8}  {:>9}  {:>7}  {:>7}  {:>7}  {:>8}",
        "Seed", "Trucks", "Orders", "Expedite", "Standard", "Preempts", "Stockouts", "Dock(m)",
        "Fork(m)", "Pick(m)", "Cleared",
    );
    println!("{}", "-".repeat(100));

    for r in &results {
        println!(
            "{:>4}  {:>7}  {:>7}  {:>8}  {:>8}  {:>8}  {:>9}  {:>7.1}  {:>7.1}  {:>7.1}  {:>8.1}",
            r.seed,
            r.trucks,
            r.orders,
            r.completed_expedite,
            r.completed_standard,
            r.putaway_preemptions,
            r.stockouts,
            r.mean_dock_wait,
            r.mean_fork_wait,
            r.mean_pick_wait,
            r.day_cleared_at,
        );
    }

    let n = results.len() as f64;
    let mean = |f: fn(&SimResult) -> f64| results.iter().map(f).sum::<f64>() / n;

    println!("{}", "-".repeat(100));
    println!(
        "{:>4}  {:>7.1}  {:>7.1}  {:>8.1}  {:>8.1}  {:>8.1}  {:>9.1}  {:>7.1}  {:>7.1}  {:>7.1}  {:>8.1}",
        "mean",
        mean(|r| r.trucks as f64),
        mean(|r| r.orders as f64),
        mean(|r| r.completed_expedite as f64),
        mean(|r| r.completed_standard as f64),
        mean(|r| r.putaway_preemptions as f64),
        mean(|r| r.stockouts as f64),
        mean(|r| r.mean_dock_wait),
        mean(|r| r.mean_fork_wait),
        mean(|r| r.mean_pick_wait),
        mean(|r| r.day_cleared_at),
    );

    println!(
        "\nPer-run logs written to {}/warehouse_run_00.log … warehouse_run_09.log",
        log_dir().display(),
    );
}

//! Machine-readable comparison runner for the simu-vs-SimPy harness.
//!
//! Runs one of several models for a range of seeds and prints a single JSON
//! object on stdout describing per-seed output metrics plus wall-clock time and
//! an event count. The Python side (`compare/models/*.py`) emits the **same**
//! JSON contract so the harness can compare distributions across many seeds and
//! measure relative performance. See `compare/README.md`.
//!
//! Usage:
//!   cargo run --release --example compare -- \
//!       --model mm1 --seeds 1000 --n 1000 --lambda 0.9 --mu 1.0 --servers 1
//!
//! Models:
//!   mm1       M/M/1 queue (single server)
//!   mmc       M/M/c queue (`--servers c`)
//!   priority  two-class priority queue (PriorityResource, 50/50 split)
//!   hospital  faithful port of examples/hospital.rs (ignores --n/--lambda/--mu)
//!
//! JSON is hand-written (no serde), mirroring the JSONL approach already used by
//! examples/warehouse.rs.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Instant;

use rand::Rng;
use rand_distr::Exp;
use simu::{any_of, Container, EnvHandle, EventTrigger, PriorityResource, Resource, SimEnv};

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

struct Args {
    model: String,
    seeds: u64,
    n: u64,
    lambda: f64,
    mu: f64,
    servers: usize,
}

fn parse_args() -> Args {
    let mut args = Args {
        model: "mm1".to_string(),
        seeds: 1,
        n: 1000,
        lambda: 0.9,
        mu: 1.0,
        servers: 1,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let key = argv[i].as_str();
        let val = argv.get(i + 1).cloned().unwrap_or_default();
        match key {
            "--model" => args.model = val,
            "--seeds" => args.seeds = val.parse().expect("--seeds expects an integer"),
            "--n" => args.n = val.parse().expect("--n expects an integer"),
            "--lambda" => args.lambda = val.parse().expect("--lambda expects a float"),
            "--mu" => args.mu = val.parse().expect("--mu expects a float"),
            "--servers" => args.servers = val.parse().expect("--servers expects an integer"),
            other => panic!("unknown argument: {other}"),
        }
        i += 2;
    }
    args
}

// ---------------------------------------------------------------------------
// Tiny JSON helpers (numbers + string-keyed objects only)
// ---------------------------------------------------------------------------

/// Format an f64 so it always round-trips and never emits bare `NaN`/`inf`
/// (which are not valid JSON). Non-finite values become `0`.
fn num(x: f64) -> String {
    if x.is_finite() {
        format!("{x}")
    } else {
        "0".to_string()
    }
}

/// Build a JSON object body from `(key, value)` pairs already-formatted as JSON.
fn obj(pairs: &[(&str, String)]) -> String {
    let body: Vec<String> = pairs
        .iter()
        .map(|(k, v)| format!("\"{k}\":{v}"))
        .collect();
    format!("{{{}}}", body.join(","))
}

// ---------------------------------------------------------------------------
// Queue models (M/M/1 and M/M/c share the same code; mm1 == servers 1)
// ---------------------------------------------------------------------------

struct Rec {
    wait: f64,
    service: f64,
    departure: f64,
}

async fn queue_customer(
    env: EnvHandle,
    res: Resource,
    arrival: f64,
    service: f64,
    recs: Rc<RefCell<Vec<Rec>>>,
) {
    let guard = res.request().await;
    let wait = env.now() - arrival;
    env.timeout(service).await;
    drop(guard);
    let departure = env.now();
    recs.borrow_mut().push(Rec { wait, service, departure });
}

async fn queue_arrivals(
    env: EnvHandle,
    res: Resource,
    n: u64,
    lambda: f64,
    mu: f64,
    recs: Rc<RefCell<Vec<Rec>>>,
) {
    let arr = Exp::new(lambda).unwrap();
    let srv = Exp::new(mu).unwrap();
    for _ in 0..n {
        let inter_arrival = env.rng().sample(arr);
        env.timeout(inter_arrival).await;
        let arrival = env.now();
        let service = env.rng().sample(srv);
        env.spawn(queue_customer(
            env.clone(),
            res.clone(),
            arrival,
            service,
            recs.clone(),
        ));
    }
}

/// One queue replication. Returns the per-seed JSON object body.
fn run_queue(seed: u64, n: u64, lambda: f64, mu: f64, servers: usize) -> String {
    let mut env = SimEnv::with_seed(seed);
    let res = Resource::new(servers);
    let recs: Rc<RefCell<Vec<Rec>>> = Rc::new(RefCell::new(Vec::with_capacity(n as usize)));

    env.spawn(queue_arrivals(
        env.handle(),
        res.clone(),
        n,
        lambda,
        mu,
        recs.clone(),
    ));
    env.run();

    let recs = recs.borrow();
    let count = recs.len() as f64;
    let t_end = recs.iter().map(|r| r.departure).fold(0.0_f64, f64::max);
    let sum_wait: f64 = recs.iter().map(|r| r.wait).sum();
    let sum_service: f64 = recs.iter().map(|r| r.service).sum();
    let sum_sojourn: f64 = recs.iter().map(|r| r.wait + r.service).sum();

    let mean_wait = if count > 0.0 { sum_wait / count } else { 0.0 };
    let mean_sojourn = if count > 0.0 { sum_sojourn / count } else { 0.0 };
    // Mean number in system L = area under N(t) / horizon = sum(sojourn) / T.
    let mean_queue = if t_end > 0.0 { sum_sojourn / t_end } else { 0.0 };
    let utilization = if t_end > 0.0 {
        sum_service / (servers as f64 * t_end)
    } else {
        0.0
    };
    let throughput = if t_end > 0.0 { count / t_end } else { 0.0 };

    obj(&[
        ("mean_wait", num(mean_wait)),
        ("mean_sojourn", num(mean_sojourn)),
        ("mean_queue", num(mean_queue)),
        ("utilization", num(utilization)),
        ("throughput", num(throughput)),
    ])
}

// ---------------------------------------------------------------------------
// Priority queue (two classes, 50/50 split)
// ---------------------------------------------------------------------------

struct PRec {
    prio: u32,
    wait: f64,
}

async fn prio_customer(
    env: EnvHandle,
    res: PriorityResource,
    prio: u32,
    arrival: f64,
    service: f64,
    recs: Rc<RefCell<Vec<PRec>>>,
) {
    let guard = res.request(prio).await;
    let wait = env.now() - arrival;
    env.timeout(service).await;
    drop(guard);
    recs.borrow_mut().push(PRec { prio, wait });
}

async fn prio_arrivals(
    env: EnvHandle,
    res: PriorityResource,
    n: u64,
    lambda: f64,
    mu: f64,
    recs: Rc<RefCell<Vec<PRec>>>,
) {
    let arr = Exp::new(lambda).unwrap();
    let srv = Exp::new(mu).unwrap();
    for _ in 0..n {
        let inter_arrival = env.rng().sample(arr);
        env.timeout(inter_arrival).await;
        let arrival = env.now();
        let service = env.rng().sample(srv);
        let prio: u32 = if env.rng().gen::<f64>() < 0.5 { 0 } else { 1 };
        env.spawn(prio_customer(
            env.clone(),
            res.clone(),
            prio,
            arrival,
            service,
            recs.clone(),
        ));
    }
}

fn run_priority(seed: u64, n: u64, lambda: f64, mu: f64, servers: usize) -> String {
    let mut env = SimEnv::with_seed(seed);
    let res = PriorityResource::new(servers);
    let recs: Rc<RefCell<Vec<PRec>>> = Rc::new(RefCell::new(Vec::with_capacity(n as usize)));

    env.spawn(prio_arrivals(
        env.handle(),
        res.clone(),
        n,
        lambda,
        mu,
        recs.clone(),
    ));
    env.run();

    let recs = recs.borrow();
    let mean_of = |class: Option<u32>| -> f64 {
        let xs: Vec<f64> = recs
            .iter()
            .filter(|r| class.is_none_or(|c| r.prio == c))
            .map(|r| r.wait)
            .collect();
        if xs.is_empty() {
            0.0
        } else {
            xs.iter().sum::<f64>() / xs.len() as f64
        }
    };

    obj(&[
        ("mean_wait_high", num(mean_of(Some(0)))),
        ("mean_wait_low", num(mean_of(Some(1)))),
        ("mean_wait_all", num(mean_of(None))),
    ])
}

// ---------------------------------------------------------------------------
// Container (mixed-size FIFO stress test)
//
// A producer drips fixed-size puts into an initially-empty container while
// consumers draw either a LARGE or SMALL amount. Under contention a large draw
// at the head of the queue cannot be satisfied until enough has accumulated;
// the question is whether a later small draw that *currently fits* must wait
// behind it (strict FIFO) or may be granted out of turn. This isolates the
// blood-bank divergence seen in the hospital model. Self-contained constants;
// ignores --lambda/--mu/--servers.
// ---------------------------------------------------------------------------

const C_CAP: f64 = 1000.0;
const C_ARRIVAL_SCALE: f64 = 1.0; // mean inter-arrival
const C_SMALL: f64 = 2.0;
const C_LARGE: f64 = 20.0;
const C_P_LARGE: f64 = 0.3;
const C_PUT_AMT: f64 = 7.0;
const C_PUT_INTERVAL: f64 = 1.0;

struct CRec {
    large: bool,
    wait: f64,
}

async fn c_consumer(
    env: EnvHandle,
    cont: Container,
    amount: f64,
    large: bool,
    arrival: f64,
    recs: Rc<RefCell<Vec<CRec>>>,
) {
    cont.get(amount).await;
    let wait = env.now() - arrival;
    recs.borrow_mut().push(CRec { large, wait });
}

async fn c_arrivals(env: EnvHandle, cont: Container, n: u64, recs: Rc<RefCell<Vec<CRec>>>) {
    let arr = Exp::new(1.0 / C_ARRIVAL_SCALE).unwrap();
    for _ in 0..n {
        let inter_arrival = env.rng().sample(arr);
        env.timeout(inter_arrival).await;
        let arrival = env.now();
        let large = env.rng().gen::<f64>() < C_P_LARGE;
        let amount = if large { C_LARGE } else { C_SMALL };
        env.spawn(c_consumer(
            env.clone(),
            cont.clone(),
            amount,
            large,
            arrival,
            recs.clone(),
        ));
    }
}

async fn c_producer(env: EnvHandle, cont: Container, puts: u64) {
    for _ in 0..puts {
        env.timeout(C_PUT_INTERVAL).await;
        cont.put(C_PUT_AMT).await;
    }
}

fn run_container(seed: u64, n: u64) -> String {
    let mut env = SimEnv::with_seed(seed);
    let cont = Container::new(C_CAP, 0.0);
    let recs: Rc<RefCell<Vec<CRec>>> = Rc::new(RefCell::new(Vec::with_capacity(n as usize)));

    env.spawn(c_arrivals(env.handle(), cont.clone(), n, recs.clone()));
    env.spawn(c_producer(env.handle(), cont.clone(), n));
    env.run();

    let recs = recs.borrow();
    let mean_of = |which: Option<bool>| -> f64 {
        let xs: Vec<f64> = recs
            .iter()
            .filter(|r| which.is_none_or(|w| r.large == w))
            .map(|r| r.wait)
            .collect();
        if xs.is_empty() {
            0.0
        } else {
            xs.iter().sum::<f64>() / xs.len() as f64
        }
    };
    let served = recs.len() as f64;

    obj(&[
        ("mean_wait_all", num(mean_of(None))),
        ("mean_wait_small", num(mean_of(Some(false)))),
        ("mean_wait_large", num(mean_of(Some(true)))),
        ("served", num(served)),
    ])
}

// ---------------------------------------------------------------------------
// Hospital (port of examples/hospital.rs; statistical-comparison policy)
//
// Differences vs. the showcase example (deliberate, mirrored on the SimPy side):
//   * no per-run log files;
//   * `ed_cleared_at` is the latest patient discharge time (tracked directly),
//     instead of an AllOf join — equivalent and simpler;
//   * no forced-completion deadline: patients that block on a depleted blood
//     bank simply never discharge; the run ends when the event queue drains.
//     Both tools apply this identical policy, so the comparison stays fair.
// ---------------------------------------------------------------------------

const H_SIM_DURATION: f64 = 480.0;
const H_ARRIVAL_RATE: f64 = 1.0 / 8.0;
const H_TRIAGE_DURATION: f64 = 5.0;
const H_MEAN_TREATMENT: f64 = 20.0;
const H_CRITICAL_PROB: f64 = 0.3;
const H_BLOOD_CAPACITY: f64 = 100.0;
const H_BLOOD_INITIAL: f64 = 60.0;
const H_BLOOD_RESTOCK: f64 = 20.0;
const H_RESTOCK_INTERVAL: f64 = 60.0;
const H_BLOOD_CRITICAL: f64 = 10.0;
const H_BLOOD_STANDARD: f64 = 2.0;

#[derive(Default)]
struct HStats {
    critical_treated: u32,
    standard_treated: u32,
    early_discharged: u32,
    blood_bank_waits: u32,
    total_nurse_wait: f64,
    total_bed_wait: f64,
    total_blood_wait: f64,
    ed_cleared_at: f64,
}

#[derive(Clone)]
struct HCtx {
    nurse: PriorityResource,
    beds: Resource,
    blood_bank: Container,
    eviction_map: Rc<RefCell<BTreeMap<u32, EventTrigger>>>,
    stats: Rc<RefCell<HStats>>,
}

async fn h_restock(env: EnvHandle, ctx: HCtx) {
    loop {
        env.timeout(H_RESTOCK_INTERVAL).await;
        if env.now() > H_SIM_DURATION {
            break;
        }
        ctx.blood_bank.put(H_BLOOD_RESTOCK).await;
    }
}

async fn h_arrivals(env: EnvHandle, ctx: HCtx) {
    let arrivals_dist = Exp::new(H_ARRIVAL_RATE).unwrap();
    let treatment_dist = Exp::new(1.0 / H_MEAN_TREATMENT).unwrap();
    let mut patient_id = 1_u32;
    loop {
        let inter_arrival = env.rng().sample(arrivals_dist);
        env.timeout(inter_arrival).await;
        if env.now() > H_SIM_DURATION {
            break;
        }
        let treatment = env.rng().sample(treatment_dist);
        let is_critical = env.rng().gen::<f64>() < H_CRITICAL_PROB;
        let triage = if is_critical { 0_u32 } else { 1_u32 };
        env.spawn(h_patient(env.clone(), patient_id, triage, treatment, ctx.clone()));
        patient_id += 1;
    }
}

async fn h_patient(env: EnvHandle, id: u32, triage: u32, treatment: f64, ctx: HCtx) {
    let blood_units = if triage == 0 {
        H_BLOOD_CRITICAL
    } else {
        H_BLOOD_STANDARD
    };

    // Triage nurse (priority).
    let nurse_wait_start = env.now();
    let nurse = ctx.nurse.request(triage).await;
    let nurse_wait = env.now() - nurse_wait_start;
    env.timeout(H_TRIAGE_DURATION).await;
    drop(nurse);

    // Blood draw.
    let blood_wait_start = env.now();
    ctx.blood_bank.get(blood_units).await;
    let blood_wait = env.now() - blood_wait_start;
    if blood_wait > 0.0 {
        ctx.stats.borrow_mut().blood_bank_waits += 1;
    }

    // Critical patient evicts the longest-admitted patient if beds are full.
    if triage == 0 && ctx.beds.in_use() >= ctx.beds.capacity() {
        let victim_id = ctx.eviction_map.borrow().keys().next().copied();
        if let Some(vid) = victim_id {
            if let Some(trigger) = ctx.eviction_map.borrow_mut().remove(&vid) {
                trigger.fire();
            }
        }
    }

    // Wait for a bed.
    let bed_wait_start = env.now();
    let bed = ctx.beds.request().await;
    let bed_wait = env.now() - bed_wait_start;
    let admitted_at = env.now();

    // Register personal eviction signal, race treatment vs. eviction.
    let (my_trigger, my_signal) = env.event();
    ctx.eviction_map.borrow_mut().insert(id, my_trigger);
    any_of![env.timeout(treatment), my_signal].await;
    let was_early = env.now() < admitted_at + treatment;
    ctx.eviction_map.borrow_mut().remove(&id);
    drop(bed);

    let mut s = ctx.stats.borrow_mut();
    if triage == 0 {
        s.critical_treated += 1;
    } else {
        s.standard_treated += 1;
    }
    if was_early {
        s.early_discharged += 1;
    }
    s.total_nurse_wait += nurse_wait;
    s.total_bed_wait += bed_wait;
    s.total_blood_wait += blood_wait;
    if env.now() > s.ed_cleared_at {
        s.ed_cleared_at = env.now();
    }
}

fn run_hospital(seed: u64) -> String {
    let mut env = SimEnv::with_seed(seed);
    let ctx = HCtx {
        nurse: PriorityResource::new(1),
        beds: Resource::new(3),
        blood_bank: Container::new(H_BLOOD_CAPACITY, H_BLOOD_INITIAL),
        eviction_map: Rc::new(RefCell::new(BTreeMap::new())),
        stats: Rc::new(RefCell::new(HStats::default())),
    };
    env.spawn(h_restock(env.handle(), ctx.clone()));
    env.spawn(h_arrivals(env.handle(), ctx.clone()));
    env.run();

    let s = ctx.stats.borrow();
    let total = (s.critical_treated + s.standard_treated) as f64;
    let mean = |sum: f64| if total > 0.0 { sum / total } else { 0.0 };

    obj(&[
        ("critical_treated", num(s.critical_treated as f64)),
        ("standard_treated", num(s.standard_treated as f64)),
        ("early_discharged", num(s.early_discharged as f64)),
        ("blood_bank_waits", num(s.blood_bank_waits as f64)),
        ("mean_nurse_wait", num(mean(s.total_nurse_wait))),
        ("mean_bed_wait", num(mean(s.total_bed_wait))),
        ("mean_blood_wait", num(mean(s.total_blood_wait))),
        ("ed_cleared_at", num(s.ed_cleared_at)),
    ])
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let args = parse_args();

    // Event count uses an identical, model-specific definition on both sides so
    // events/sec is comparable across tools (see compare/README.md).
    let events = Cell::new(0_u64);

    let start = Instant::now();
    let mut per_seed: Vec<String> = Vec::with_capacity(args.seeds as usize);
    for seed in 0..args.seeds {
        let body = match args.model.as_str() {
            "mm1" => {
                events.set(events.get() + 2 * args.n); // arrival + departure per customer
                run_queue(seed, args.n, args.lambda, args.mu, 1)
            }
            "mmc" => {
                events.set(events.get() + 2 * args.n);
                run_queue(seed, args.n, args.lambda, args.mu, args.servers)
            }
            "priority" => {
                events.set(events.get() + 2 * args.n);
                run_priority(seed, args.n, args.lambda, args.mu, args.servers)
            }
            "container" => {
                let body = run_container(seed, args.n);
                // events := arrivals (n) + completed gets (served).
                events.set(events.get() + args.n + field_u64(&body, "served"));
                body
            }
            "hospital" => {
                let body = run_hospital(seed);
                // events := completed patients (two transitions each).
                events.set(events.get() + 2 * count_completed(&body));
                body
            }
            other => panic!("unknown model: {other}"),
        };
        per_seed.push(body);
    }
    let wall_secs = start.elapsed().as_secs_f64();

    let meta = obj(&[
        ("tool", "\"simu\"".to_string()),
        ("model", format!("\"{}\"", args.model)),
        ("seeds", args.seeds.to_string()),
        ("n", args.n.to_string()),
        ("lambda", num(args.lambda)),
        ("mu", num(args.mu)),
        ("servers", args.servers.to_string()),
        ("events", events.get().to_string()),
        ("wall_secs", num(wall_secs)),
        ("per_seed", format!("[{}]", per_seed.join(","))),
    ]);
    println!("{meta}");
}

/// Read a numeric field out of an already-formatted per-seed JSON body so the
/// event tally stays in lockstep with the metrics actually emitted.
fn field_u64(body: &str, name: &str) -> u64 {
    body.split(&format!("\"{name}\":"))
        .nth(1)
        .and_then(|s| s.split([',', '}']).next())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map(|x| x as u64)
        .unwrap_or(0)
}

fn count_completed(body: &str) -> u64 {
    field_u64(body, "critical_treated") + field_u64(body, "standard_treated")
}

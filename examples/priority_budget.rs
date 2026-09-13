// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! PoC: priority + budget scheduling (checklist item #3).
//!
//! Two agent classes share one LLM provider with [`PROVIDER_SLOTS`]
//! concurrent slots:
//!
//! - **premium** — high priority (0), small budget ($0.05): interactive
//!   agents that must start quickly but may only spend a little.
//! - **bulk** — low priority (10), large budget ($0.20): batch agents that
//!   may spend a lot but can wait.
//!
//! Two *orthogonal* policies, and where each lives:
//!
//! - **Priority** is a *queueing* policy, so it uses a kernel primitive:
//!   [`PriorityResource`] (vs. a plain FIFO [`Resource`] for comparison).
//! - **Budget** is *accounting*, so it is pure model-layer code: each agent
//!   tracks its own spend and stops when the next request would exceed its
//!   budget. No budget concept exists in the kernel.
//!
//! The same seeded workload is replayed under both queueing policies, so the
//! per-class comparison isolates the effect of priority scheduling. Run:
//!
//! ```sh
//! cargo run --release --example priority_budget
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use rand::Rng;
use simcore::{PriorityResource, PriorityResourceGuard, Resource, ResourceGuard, SimEnv};

const PROVIDER_SLOTS: usize = 2;
const PLANNED_REQUESTS: usize = 12;
const PRICE_PER_TOKEN: f64 = 0.000_01; // dollars per token

struct AgentClass {
    name: &'static str,
    count: u32,
    priority: u32,
    budget: f64,
}

const CLASSES: [AgentClass; 2] = [
    AgentClass { name: "premium", count: 4, priority: 0, budget: 0.05 },
    AgentClass { name: "bulk", count: 6, priority: 10, budget: 0.20 },
];

/// One precomputed request: think time before it, token size, service time.
struct RequestPlan {
    think: f64,
    tokens: u32,
    latency: f64,
}

#[derive(Clone, Copy, Default)]
struct ClassStats {
    completed: u32,
    wait_total: f64,
    wait_max: f64,
    cost_total: f64,
}

/// Queueing policy under test: FIFO kernel resource vs. priority resource.
enum Pool {
    Fifo(Resource),
    Prio(PriorityResource),
}

/// Owns the acquired unit; dropping it releases the slot either way.
/// The fields are never read — holding the guard *is* the acquisition.
enum Slot {
    #[allow(dead_code)]
    F(ResourceGuard),
    #[allow(dead_code)]
    P(PriorityResourceGuard),
}

impl Pool {
    async fn acquire(&self, priority: u32) -> Slot {
        match self {
            Pool::Fifo(r) => Slot::F(r.request().await),
            Pool::Prio(r) => Slot::P(r.request(priority).await),
        }
    }
}

/// Replay the identical seeded workload under one queueing policy and
/// return per-class statistics.
fn run_policy(pool: Pool) -> [ClassStats; CLASSES.len()] {
    let mut env = SimEnv::with_seed(42);

    // Precompute the whole workload before spawning, so both policies see
    // exactly the same think times, token sizes, and latencies.
    let mut plans: Vec<Vec<Vec<RequestPlan>>> = Vec::new(); // [class][agent][request]
    {
        let h = env.handle();
        let mut rng = h.rng();
        for class in &CLASSES {
            let mut class_plans = Vec::new();
            for _ in 0..class.count {
                let agent_plan = (0..PLANNED_REQUESTS)
                    .map(|_| RequestPlan {
                        think: 0.3 + 0.6 * rng.random::<f64>(),
                        tokens: 1_000 + rng.random_range(0..1_000),
                        latency: 0.4 + 0.6 * rng.random::<f64>(),
                    })
                    .collect();
                class_plans.push(agent_plan);
            }
            plans.push(class_plans);
        }
    }

    let stats: Vec<Rc<RefCell<ClassStats>>> = (0..CLASSES.len())
        .map(|_| Rc::new(RefCell::new(ClassStats::default())))
        .collect();

    for (class_idx, class) in CLASSES.iter().enumerate() {
        for agent_plans in &plans[class_idx] {
            let h = env.handle();
            // Cheap Rc clones — all agents share the same pool.
            let pool = match &pool {
                Pool::Fifo(r) => Pool::Fifo(r.clone()),
                Pool::Prio(r) => Pool::Prio(r.clone()),
            };
            let stats = Rc::clone(&stats[class_idx]);
            let (priority, budget) = (class.priority, class.budget);
            let agent_plans = agent_plans.iter().map(|p| (p.think, p.tokens, p.latency)).collect::<Vec<_>>();
            env.spawn(async move {
                let mut spent = 0.0;
                for (think, tokens, latency) in agent_plans {
                    h.timeout(think).await; // paced arrival

                    // --- model-layer budget gate --------------------------
                    // The kernel knows nothing about money; the agent simply
                    // stops when the next request would overrun its budget.
                    let cost = f64::from(tokens) * PRICE_PER_TOKEN;
                    if spent + cost > budget {
                        break;
                    }

                    // --- kernel-layer priority queueing -------------------
                    let queued_at = h.now();
                    let _slot = pool.acquire(priority).await;
                    let wait = h.now() - queued_at;

                    h.timeout(latency).await; // the provider call itself

                    spent += cost;
                    let mut s = stats.borrow_mut();
                    s.completed += 1;
                    s.wait_total += wait;
                    s.wait_max = s.wait_max.max(wait);
                    s.cost_total += cost;
                }
            });
        }
    }

    env.run();
    let mut out = [ClassStats::default(); CLASSES.len()];
    for (i, s) in stats.iter().enumerate() {
        out[i] = *s.borrow();
    }
    out
}

fn report(label: &str, stats: &[ClassStats; CLASSES.len()]) {
    println!("--- {label} ---");
    println!(
        "{:<9} {:>10} {:>10} {:>10} {:>10}",
        "class", "completed", "avg wait", "max wait", "cost"
    );
    for (class, s) in CLASSES.iter().zip(stats) {
        let avg_wait = if s.completed > 0 {
            s.wait_total / f64::from(s.completed)
        } else {
            0.0
        };
        println!(
            "{:<9} {:>10} {:>10.3} {:>10.3} {:>9.4}$",
            class.name, s.completed, avg_wait, s.wait_max, s.cost_total
        );
    }
    println!();
}

fn main() {
    let fifo = run_policy(Pool::Fifo(Resource::new(PROVIDER_SLOTS)));
    let prio = run_policy(Pool::Prio(PriorityResource::new(PROVIDER_SLOTS)));

    println!("workload: {} premium agents (prio {}, ${:.2} budget), {} bulk agents (prio {}, ${:.2} budget)",
        CLASSES[0].count, CLASSES[0].priority, CLASSES[0].budget,
        CLASSES[1].count, CLASSES[1].priority, CLASSES[1].budget);
    println!("provider: {PROVIDER_SLOTS} slots, ${PRICE_PER_TOKEN}/token, identical seeded workload\n");

    report("policy: FIFO (kernel Resource)", &fifo);
    report("policy: priority (kernel PriorityResource)", &prio);

    let premium_speedup = (fifo[0].wait_total / f64::from(fifo[0].completed).max(1.0))
        / (prio[0].wait_total / f64::from(prio[0].completed).max(1.0)).max(1e-9);
    println!("--- reading ---");
    println!("premium avg wait: FIFO {:.3} -> priority {:.3} ({premium_speedup:.1}x lower)",
        fifo[0].wait_total / f64::from(fifo[0].completed).max(1.0),
        prio[0].wait_total / f64::from(prio[0].completed).max(1.0));
    println!("completions and cost are identical across policies: budget (model layer)");
    println!("caps spend regardless of queueing; priority only reallocates *who waits*.");
}

// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use simu::SimEnv;
use simu::{Container, PreemptiveResource, PriorityResource, Resource};

// ---------------------------------------------------------------------------
// 1. timeout_throughput — core executor baseline
//
// Spawn N processes each doing timeout(i as f64).await. Measures raw
// event-queue (BinaryHeap push/pop) + executor throughput.
// ---------------------------------------------------------------------------

fn timeout_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("timeout_throughput");

    for n in [1_000_u64, 10_000, 100_000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                let mut env = SimEnv::with_seed(0);
                for i in 0..n {
                    let h = env.handle();
                    env.spawn(async move {
                        h.timeout(i as f64).await;
                    });
                }
                env.run();
            });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 2. resource_contention — FIFO queue under load
//
// N processes queue for a single-capacity resource. Each holds it for 1 tick.
// Measures ResourceRequest waker registration and WaitQueue heap push/pop plus
// the ResourceGuard::Drop direct-handoff chain.
// ---------------------------------------------------------------------------

fn resource_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("resource_contention");

    for n in [100_u32, 1_000, 10_000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                let mut env = SimEnv::with_seed(0);
                let resource = Resource::new(1);

                for _ in 0..n {
                    let h = env.handle();
                    let r = resource.clone();
                    env.spawn(async move {
                        let _guard = r.request().await;
                        h.timeout(1.0).await;
                    });
                }

                env.run();
            });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 3. event_broadcast — multi-waiter dispatch
//
// N processes await the same EventAwaitable. One process fires at t=1.
// All N wake simultaneously. Measures waker registration and drain dispatch.
// ---------------------------------------------------------------------------

fn event_broadcast(c: &mut Criterion) {
    let mut group = c.benchmark_group("event_broadcast");

    for n in [100_u32, 1_000, 10_000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                let mut env = SimEnv::with_seed(0);
                let (trigger, awaitable) = env.event();

                for _ in 0..n {
                    let h = env.handle();
                    let aw = awaitable.clone();
                    env.spawn(async move {
                        aw.await;
                        let _ = h.now();
                    });
                }

                {
                    let h = env.handle();
                    env.spawn(async move {
                        h.timeout(1.0).await;
                        trigger.fire();
                    });
                }

                env.run();
            });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 4. mixed_workload — realistic simulation
//
// N patients each: acquire nurse (cap 1) → hold 5 ticks → acquire bed (cap 3)
// → hold 20 ticks. Tests real-world interaction between all primitives.
// ---------------------------------------------------------------------------

fn run_mixed(n: u32) {
    let mut env = SimEnv::with_seed(0);
    let nurse = Resource::new(1);
    let beds = Resource::new(3);

    // Arrivals: spawn one patient every 1 tick.
    {
        let nurse = nurse.clone();
        let beds = beds.clone();
        let h = env.handle();

        // Spawn patients directly to avoid capturing mutable env inside async.
        for i in 0..n {
            let h = h.clone();
            let nurse = nurse.clone();
            let beds = beds.clone();
            env.spawn(async move {
                h.timeout(i as f64).await; // arrival time
                let _nurse_guard = nurse.request().await;
                h.timeout(5.0).await;
                drop(_nurse_guard);
                let _bed_guard = beds.request().await;
                h.timeout(20.0).await;
            });
        }
    }

    env.run();
}

fn mixed_workload(c: &mut Criterion) {
    let mut group = c.benchmark_group("mixed_workload");

    for n in [100_u32, 1_000, 10_000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| run_mixed(n));
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 5. monte_carlo_scaling — parallelism efficiency
//
// Run K independent copies of mixed_workload(100) via monte_carlo::run.
// Measures how throughput scales with thread count.
// ---------------------------------------------------------------------------

fn monte_carlo_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("monte_carlo_scaling");

    for k in [1_u64, 2, 4, 8] {
        group.bench_with_input(BenchmarkId::from_parameter(k), &k, |b, &k| {
            b.iter(|| {
                simu::monte_carlo::run(0..k, |_seed| run_mixed(100));
            });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 6. priority_contention — priority heap ordering under load
//
// N processes queue for a single-capacity PriorityResource at alternating
// priorities. Measures the BinaryHeap<Entry<u32>> push/pop ordering cost on top
// of the wake-and-handoff chain.
// ---------------------------------------------------------------------------

fn priority_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("priority_contention");

    for n in [100_u32, 1_000, 10_000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                let mut env = SimEnv::with_seed(0);
                let resource = PriorityResource::new(1);

                for i in 0..n {
                    let h = env.handle();
                    let r = resource.clone();
                    let prio = i % 4; // spread across four priority levels
                    env.spawn(async move {
                        let _guard = r.request(prio).await;
                        h.timeout(1.0).await;
                    });
                }

                env.run();
            });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 7. preemptive_contention — eviction path under load
//
// Capacity-2 PreemptiveResource. Half the requests are high priority (0) and
// half low (1), so high-priority arrivals actively evict low-priority holders.
// Exercises the victim scan + holder registry alongside the queue.
// ---------------------------------------------------------------------------

fn preemptive_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("preemptive_contention");

    for n in [100_u32, 1_000, 10_000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                let mut env = SimEnv::with_seed(0);
                let resource = PreemptiveResource::new(2);

                for i in 0..n {
                    let h = env.handle();
                    let r = resource.clone();
                    let prio = i % 2; // alternate high (0) / low (1)
                    env.spawn(async move {
                        let guard = r.request(prio).await;
                        // Race the hold against a possible preemption.
                        simu::any_of![h.timeout(1.0), guard.preempted()].await;
                    });
                }

                env.run();
            });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 8. container_throughput — put/get cascade under load
//
// N producers each put 1 unit and N consumers each get 1 unit into a shared
// Container. Measures the head-of-line FIFO cascade (wake_get/put_waiters).
// ---------------------------------------------------------------------------

fn container_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("container_throughput");

    for n in [100_u32, 1_000, 10_000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                let mut env = SimEnv::with_seed(0);
                let tank = Container::new(f64::from(n) + 1.0, 0.0);

                // Consumers block first (level starts at 0), then producers
                // trickle material in, driving the get-cascade.
                for _ in 0..n {
                    let c = tank.clone();
                    env.spawn(async move {
                        c.get(1.0).await;
                    });
                }
                for _ in 0..n {
                    let h = env.handle();
                    let c = tank.clone();
                    env.spawn(async move {
                        h.timeout(1.0).await;
                        c.put(1.0).await;
                    });
                }

                env.run();
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    timeout_throughput,
    resource_contention,
    event_broadcast,
    mixed_workload,
    monte_carlo_scaling,
    priority_contention,
    preemptive_contention,
    container_throughput,
);
criterion_main!(benches);

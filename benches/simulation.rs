use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use simu::env::SimEnv;
use simu::Resource;

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
// Measures ResourceRequest waker registration and VecDeque push/pop costs.
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

criterion_group!(
    benches,
    timeout_throughput,
    resource_contention,
    event_broadcast,
    mixed_workload,
    monte_carlo_scaling,
);
criterion_main!(benches);

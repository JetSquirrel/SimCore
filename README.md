# simu

A Rust library for Discrete Event Simulation (DES), inspired by Python's SimPy but designed to be idiomatic Rust, high-performance, and scalable.

**Goals:**

- Model complex, process-oriented simulations (e.g. hospital operations, logistics, queuing systems)
- Support thousands of concurrent simulation processes with low overhead
- Reproducible results via seeded RNG
- Monte Carlo parallelism across independent simulation runs using OS threads

For full technical details — architecture, API design, resource model, and roadmap — see [SPEC.md](SPEC.md).  
For the implementation roadmap and feature status — see [PLAN.md](PLAN.md).  
For test strategy, coverage, and benchmark groups — see [TESTING.md](TESTING.md).

## Quick start

```rust
use simu::env::SimEnv;
use simu::Resource;

let mut env = SimEnv::with_seed(42);
let machine = Resource::new(1);

for i in 1..=3_u32 {
    let h = env.handle();
    let m = machine.clone();
    env.spawn(async move {
        let _guard = m.request().await;   // queue for machine
        h.timeout(2.0).await;             // hold for 2 time units
        println!("job {} done at {}", i, h.now());
    });
}

env.run();  // prints: job 1 done at 2, job 2 done at 4, job 3 done at 6
```

## Usage

Add `simu` to your `Cargo.toml`:

```toml
[dependencies]
simu = { path = "." }
```

`monte_carlo::run` works out of the box (one `std::thread` per seed). For large seed counts, enable
the optional feature to run on rayon's bounded thread pool instead:

```toml
[dependencies]
simu = { path = ".", features = ["monte-carlo"] }
```

## Core primitives

| Type | Description |
|---|---|
| `SimEnv` | Central coordinator: owns event queue, current time, processes, and seeded RNG |
| `EnvHandle` | Cloneable handle passed into processes; provides `timeout`, `event`, `rng`, `now` |
| `Timeout` | Future that resolves after a simulated delay (`h.timeout(5.0).await`) |
| `EventTrigger` / `EventAwaitable` | Paired handles for manual inter-process signalling |
| `Resource` / `ResourceGuard` | FIFO capacity-limited pool; RAII release on guard drop |
| `PriorityResource` | Priority-scheduled pool (lower number = higher priority; FIFO within a level) |
| `PreemptiveResource` / `PreemptiveGuard` | Priority pool whose in-use units can be preempted by a higher-priority request (cooperative-at-yield) |
| `Container` | Reservoir of continuous quantity (`put` / `get`, FIFO waiters) |
| `ProcessHandle<T>` | Observable spawn; `.await` for the return value, drop to detach |
| `AnyOf` / `AllOf` | Future combinators via the `any_of!` / `all_of!` macros |

## Building

```bash
cargo build
```

## Testing

```bash
cargo test                  # run all tests
cargo test <test_name>      # run a single test
```

See [TESTING.md](TESTING.md) for the full test strategy, coverage report, and benchmark guide.

## Benchmarks

```bash
cargo bench                              # all benchmark groups
cargo bench -- timeout_throughput        # one group only
cargo bench -- --save-baseline main      # save baseline
cargo bench -- --baseline main           # compare against baseline
```

HTML reports are written to `target/criterion/`. Five benchmark groups cover executor
throughput, resource contention, event broadcast, mixed workload, and Monte Carlo scaling.

## Linting

```bash
cargo clippy -- -D warnings
```

## Examples

Three end-to-end examples demonstrate every public primitive in different domains. All run 10
parallel Monte Carlo simulations, write per-run logs, and print a summary table to stdout.
See [`examples/hospital.md`](examples/hospital.md), [`examples/brewery.md`](examples/brewery.md),
and [`examples/warehouse.md`](examples/warehouse.md) for full walkthroughs (sequence diagrams,
configuration, sample output).

```bash
cargo run --example hospital --release
```

A hospital emergency department: priority-scheduled triage nurse, three beds with eviction of the
longest-admitted patient when a critical case arrives, and a blood bank modelled as a `Container`.
Logs to `target/sim-logs/run_<N>.log`.

```bash
cargo run --example brewery --release
```

A craft brewery / bio-reactor production line from the food & beverage domain: mash → boil →
ferment → condition → bottle → CIP. The QA inspector contaminates the longest-running fermentation
with a per-batch `EventTrigger`; contaminated batches preempt routine cleanups via a
`PriorityResource`. Logs to `target/sim-logs/brewery_run_<N>.log`.

```bash
cargo run --example warehouse --release
```

A distribution center from the logistics / material-handling domain: inbound trucks (unload → QC →
putaway) and outbound orders (pick → pack → load) share one small forklift fleet. The fleet is a
`PreemptiveResource` — urgent truck-side work evicts a routine putaway, whose driver parks the
pallet and finishes it later. The first example to exercise `PreemptiveResource`. Logs to
`target/sim-logs/warehouse_run_<N>.log`.

## License

TBD

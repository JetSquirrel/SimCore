# SimCore

[![CI](https://github.com/JetSquirrel/SimCore/actions/workflows/ci.yml/badge.svg)](https://github.com/JetSquirrel/SimCore/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/simcore-des.svg)](https://crates.io/crates/simcore-des)
[![docs.rs](https://docs.rs/simcore-des/badge.svg)](https://docs.rs/simcore-des)
[![License: MIT OR Apache-2.0](https://img.shields.io/crates/l/simcore-des.svg)](#license)
[![REUSE status](https://api.reuse.software/badge/github.com/JetSquirrel/SimCore)](https://api.reuse.software/info/github.com/JetSquirrel/SimCore)

**A tiny deterministic discrete-event simulation kernel for building system simulators.**

SimCore is a Rust library for Discrete Event Simulation (DES), inspired by Python's SimPy but
designed to be idiomatic Rust, high-performance, and scalable. It is a *kernel*: it provides the
deterministic event loop, process, and resource primitives — domain-specific simulators are meant
to be built on top of it.

SimCore is a hard fork of [`simu`](https://github.com/chkhm/simu) by Christoph Kuhmuench / Siemens,
licensed MIT OR Apache-2.0. The original copyright and license notices are retained unchanged
throughout the repository.

**Design values:**

- **Tiny** — a small, focused kernel you can read end-to-end; no domain assumptions baked in.
- **Deterministic** — reproducible results via seeded RNG, or a pluggable external feed
  (`SimEnv::with_source`) — e.g. the portable `SplitMix64` generator that can be re-implemented in
  another language for exact cross-engine comparison.
- **Composable** — a small set of primitives (processes, events, resources, containers) that
  combine into complex process-oriented models: hospital operations, logistics, queuing systems.
- **Rust** — thousands of concurrent simulation processes with low overhead, and Monte Carlo
  parallelism across independent runs using OS threads. Python/JS/WASM bindings are planned.

**New to SimCore?** Start with the [`tutorial` module](https://docs.rs/simcore-des) —
five short chapters modeled on SimPy's "SimPy in 10 minutes", every snippet a
running doc-test — and its four sub-60-line companion examples
(`cargo run --example intro_car`, `intro_charging`, `intro_cancellation`,
`intro_charging_station`). A signature cheat-sheet lives in [API.md](API.md).

## Installation

```toml
[dependencies]
simcore-des = "0.1"
```

The crate is published as `simcore-des` (the crates.io name `simcore` was taken) but
the library target is named `simcore`, so it is imported as `use simcore::…` — exactly
as in the examples below.

## Quick start

```rust
use simcore::{SimEnv, Resource};

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

`monte_carlo::run` works out of the box (one `std::thread` per seed). For large seed counts, enable
the optional feature to run on rayon's bounded thread pool instead:

```toml
[dependencies]
simcore-des = { version = "0.1", features = ["monte-carlo"] }
```

## Using SimCore with AI assistants

The repository ships two LLM-oriented files, kept in sync with the crate's
compile-checked doc-tests:

- [`llms.txt`](llms.txt) — a complete single-file reference: signatures, canonical
  patterns, a **SimPy → SimCore translation table**, and anti-patterns with their
  symptoms.
- [`docs/simcore-for-agents.md`](docs/simcore-for-agents.md) — a compact version designed
  to be dropped into your own project's agent context (CLAUDE.md, cursor rules, …)
  when you build simulations with SimCore.

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
| `Container` | Reservoir of continuous quantity (`put` / `get`, strict head-of-line FIFO waiters) |
| `ProcessHandle<T>` | Observable spawn; `.await` for the return value, drop to detach |
| `AnyOf` / `AllOf` | Future combinators via the `any_of!` / `all_of!` macros |

## Roadmap

SimCore is developed kernel-first, with two binding tracks out of the Rust core:

```text
simcore (Rust kernel)
   ├── python frontend ──→ sim-llm / sim-cloud / sim-xxx   (Python modeling)
   └── cloudsimrs ──→ java frontend ──→ CloudSim / CloudSim Plus ecosystem
```

1. **Stable deterministic kernel** — the current focus: a small, well-tested DES core with
   guaranteed reproducibility (event throughput and scale validated; see `examples/perf_scale.rs`).
2. **Python frontend** *(planned)*: `simcore-python` — the primary modeling language track.
   Domain simulators are built in Python on top of it: `sim-llm`, `sim-cloud`, and further
   `sim-xxx` models.
3. **CloudSim compatibility track** *(planned)*: `cloudsimrs` — CloudSim concepts
   (Datacenter / Host / VM / Cloudlet) as a Rust domain layer on SimCore, then a Java
   frontend so CloudSim / CloudSim Plus style models can run on the SimCore backend.
   Concept/API compatibility is the goal; the kernel itself stays free of cloud types.
4. **Browser / WASM** *(planned)*: `simcore-wasm` + TypeScript API, targeting interactive
   simulation playgrounds in the browser.

Domain types (VMs, hosts, LLM providers, …) never enter the kernel crate — policies and
accounting live in the domain layers above it.

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

Start with the four **intro examples** (one per tutorial chapter, each under 60
lines): `intro_car`, `intro_charging`, `intro_cancellation`,
`intro_charging_station`.

Beyond those, three end-to-end examples demonstrate every public primitive in different domains. All run 10
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
`target/sim-logs/warehouse_run_<N>.log`. A browser-based visualizer for its runs lives in
[`examples/warehouse-viz/`](examples/warehouse-viz/).

## SimPy parity

`compare/` cross-checks SimCore against Python's [SimPy](https://simpy.readthedocs.io/)
as a reference oracle: identical JSON-contract models run on both engines and are
compared per seed. Both sides draw from the **same portable feed** (`SplitMix64` +
shared transforms, re-implemented in `compare/models/_feed.py`), so the queue
models are checked in **exact mode** — per-seed metrics agree to ~1e-15 — while
`hospital` stays on a distributional test for its eviction-handoff ordering
exception. Performance is compared on two axes: single-thread engine efficiency
(wall-clock / events-per-sec / peak-RSS), and a **Monte Carlo** benchmark where
SimCore fans independent replications across cores via `monte_carlo::run` while
SimPy is GIL-serialised — showing the full parallel advantage. Canonical queue
models are additionally checked against closed-form queueing theory, so neither
engine is trusted blindly.

```bash
compare/run_comparison.sh             # build Rust, set up venv, write compare/REPORT.md
compare/run_comparison.sh --no-perf   # correctness only
```

See [`compare/README.md`](compare/README.md) for methodology and
[`compare/REPORT.md`](compare/REPORT.md) for the latest results. On the queue
models SimCore runs ~10× faster at ~13× lower memory; the `Container` strict-FIFO
divergence the harness originally surfaced is now fixed.

## For contributors

See [CONTRIBUTING.md](CONTRIBUTING.md) for the contribution checklist (style,
SPDX headers, DCO sign-off, review process).

Internal design docs, aimed at people changing the library itself:

- [SPEC.md](SPEC.md) — architecture, API contracts, invariants, and roadmap (the design source of truth).
- [TESTING.md](TESTING.md) — test strategy, coverage, and benchmark groups.

## License

SimCore is a hard fork of [`simu`](https://github.com/chkhm/simu), copyright
Christoph Kuhmuench / Siemens. The original copyright and license notices are
retained unchanged in every file.

Licensed under either of

- Apache License, Version 2.0 ([LICENSES/Apache-2.0.txt](LICENSES/Apache-2.0.txt))
- MIT License ([LICENSES/MIT.txt](LICENSES/MIT.txt))

at your option — the Rust ecosystem's standard dual license. The repository is
[REUSE](https://reuse.software/)-compliant: every file carries SPDX licensing
information, verified by `reuse lint` in CI.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

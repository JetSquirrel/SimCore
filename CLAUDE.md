# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`simu` is a Rust library for Discrete Event Simulation (DES), inspired by SimPy. The MVP is fully
implemented and tested.

Companion docs:

- `SPEC.md` — the design source of truth (architecture, API contracts, invariants, roadmap).
- `API.md` — public API reference.
- `PLAN.md` — implementation plan / history.
- `TESTING.md` — test strategy and coverage notes.
- `README.md` — user-facing overview.

When changing behaviour, keep `SPEC.md` and `API.md` in sync.

## Commands

```bash
cargo build
cargo test                        # 69 passing tests across unit + integration suites
cargo test <test_name>            # run a single test
cargo run --example hospital      # ER patient-flow simulation
cargo run --example brewery       # brewery / process-automation simulation
cargo bench                       # Criterion benchmark suite (benches/simulation.rs)
cargo clippy -- -D warnings       # must stay warning-clean
```

## Source layout

```
simu/
├── Cargo.toml
├── src/
│   ├── lib.rs            # public re-exports only
│   ├── env.rs            # SimEnv + EnvHandle; owns run()/run_until() event loop
│   ├── executor/         # pub(crate), not user-facing
│   │   ├── mod.rs        # SimState: event queue, ready queue, process table, seq counter
│   │   ├── queue.rs      # ScheduledWaker, min-heap entry ordered by (time, seq)
│   │   └── waker.rs      # custom Waker that re-queues a process id on wake
│   ├── timeout.rs        # Timeout future
│   ├── event.rs          # EventTrigger / EventAwaitable (multi-waiter, fire-before-await latch)
│   ├── combinator.rs     # AnyOf, AllOf + any_of! / all_of! macros
│   ├── process.rs        # ProcessHandle<T>, spawn_with_handle
│   ├── monte_carlo.rs    # monte_carlo::run — std::thread (default) or rayon (monte-carlo feature)
│   └── resource/
│       ├── mod.rs        # Resource, ResourceRequest, ResourceGuard (FIFO)
│       ├── priority.rs   # PriorityResource (priority heap, FIFO within a level)
│       ├── container.rs  # Container (continuous quantity, FIFO put/get)
│       └── preemptive.rs # PreemptiveResource — STUB only (post-MVP, not implemented)
├── examples/
│   ├── hospital.rs / hospital.md
│   └── brewery.rs  / brewery.md
├── benches/
│   └── simulation.rs     # Criterion benchmarks
└── tests/                # 10 integration files: timeout, event, resource, priority_resource,
                          # container, combinator, process_handle, dropped_awaitable, system, …
```

`executor/` is private (`mod executor;`) with `pub(crate)` items — not user-facing.

## Architecture

### Core design: single-threaded custom async executor per `SimEnv`

- **No tokio/async-std.** The executor is hand-rolled, polling futures based on simulated time, not
  wall-clock time. The run loop lives in `env.rs` (`run` / `run_until` → `poll_ready`); the shared
  mutable state (`SimState`) lives in `executor/mod.rs`.
- `SimEnv` is `!Send + !Sync` — it lives entirely on one thread.
- Monte Carlo parallelism is achieved by spinning up independent `SimEnv` instances on OS threads
  (`monte_carlo::run`). By default it spawns one `std::thread` per seed; enabling the `monte-carlo`
  feature switches the backend to rayon's bounded thread pool. Because each `SimEnv` is built
  *inside* the per-seed closure, its `!Send` nature is never a problem.
- DES is inherently sequential within a run, so single-threaded execution gives zero sync overhead.

### `SimEnv` / `EnvHandle` split

- **`SimEnv`** owns simulation state and drives the event loop. Not `Clone`; one thread.
- **`EnvHandle`** is a `Clone`able handle (`env.handle()`) passed into spawned processes. It exposes
  `now`, `timeout`, `event`, `spawn`, and `rng`.
- Both share the same `Rc<RefCell<SimState>>` and the same `Rc<RefCell<StdRng>>`.

### Key types

| Type | Role |
|------|------|
| `SimEnv` | Central coordinator: owns event loop, current time (`f64`), process table, seeded `StdRng`. Not `Clone`. |
| `EnvHandle` | `Clone`able handle into the env, passed to processes (`now`/`timeout`/`event`/`spawn`/`rng`) |
| `Timeout` | Future that resolves after a simulated delay |
| `EventTrigger` / `EventAwaitable` | Manual inter-process signalling. `EventAwaitable` is `Clone` (multi-waiter); fire-before-await latch; `fire()` consumes the trigger |
| `Resource` / `ResourceGuard` | FIFO-queued, capacity-limited pool; RAII release on guard drop |
| `PriorityResource` | Priority-scheduled pool (lower number = higher priority; FIFO within a level) |
| `Container` | Reservoir of continuous quantity (`put` / `get`, FIFO waiters) |
| `ProcessHandle<T>` | Observable spawn (tokio-`JoinHandle`-style): `await` for the value, drop to detach. Not `Clone`. |
| `AnyOf` / `AllOf` | Future combinators; built via the `any_of!` / `all_of!` macros |

### Event queue invariant

Events are ordered by simulated time, then by insertion order (`seq_counter`) for deterministic
tie-breaking, via a `BinaryHeap<Reverse<ScheduledWaker>>`. Given the same seed and process logic, a
simulation must produce identical results.

### Resource ownership (no `Arc`/`Mutex`)

`Resource` (and `PriorityResource`, `Container`) wrap `Rc<RefCell<…State>>` internally and implement
`Clone`. Share a pool across processes by **cloning the handle** — *not* by wrapping in `Arc`. No
`Mutex` is needed because the executor is single-threaded. These types are `!Send + !Sync`, consistent
with `SimEnv`.

```rust
let machine = Resource::new(1);
let m = machine.clone();           // cheap Rc clone, same pool
env.spawn(async move { let _g = m.request().await; /* … */ });
```

(One internal exception: `SimState::ready_queue` is `Arc<Mutex<Vec<usize>>>` purely so it can be held
by `Waker` vtables, which require `Send + Sync`. It is never actually contended.)

### Load-bearing internal invariants (see `SPEC.md §4.7`)

- **`registered` flag** on every request-future prevents double-queuing across polls and stops a
  spurious re-poll from stealing capacity ahead of an earlier waiter (preserves FIFO).
- **`canceled` flag** (`Rc<Cell<bool>>`) on waiter entries: a request dropped before it is granted
  (e.g. a losing `any_of!` arm) marks itself canceled; guard-release loops and the `Container` wake
  cascade skip canceled entries — no starvation, and no material leak in `Container`.
- **`RngGuard`**: `EnvHandle::rng()` returns an `impl RngCore + '_` over a `RefMut`. Being `!Send` and
  borrowing `self`, it cannot cross an `.await` — deterministic sampling is safe by construction.
  Sample before awaiting.

### Error strategy

- Programming errors (wrong API use, e.g. zero capacity) → `panic!`
- Simulation errors → `Result`

## Dependencies

| Crate | Purpose |
|-------|---------|
| `rand` + `rand_distr` | Seeded RNG and distributions (required) |
| `rayon` | Optional `monte-carlo` feature: switches `monte_carlo::run` to rayon's bounded thread pool (default backend is `std::thread`) |
| `criterion` | Benchmark harness (dev-dependency) |

`thiserror` will be added when the first public fallible API arrives; until then the surface is
panic-on-misuse only. No async runtime dependency — the executor is self-contained.

## Status

All MVP features in `SPEC.md §5` are implemented: `SimEnv`/event queue, `Timeout`, manual `Event`
(multi-waiter + fire-before-await latch), `Resource` (FIFO + RAII guard), `PriorityResource`,
`Container`, `ProcessHandle<T>`, `AnyOf`/`AllOf` + macros, `spawn`, `run`/`run_until`, seeded RNG,
deterministic tie-breaking, `monte_carlo::run`, both examples, integration tests, and Criterion benches.

**Post-MVP (not yet implemented)** — see `SPEC.md §6`:

- `PreemptiveResource` — `src/resource/preemptive.rs` is a one-line placeholder stub only.
- `Interrupt`, `RealtimeEnvironment`, `Store` / `FilterStore`, GPU/CUDA acceleration.

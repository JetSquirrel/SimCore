# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`simu` is a Rust library for Discrete Event Simulation (DES), inspired by SimPy. The spec lives in `SPEC.md`. No code exists yet — this is the source of truth for all design decisions.

## Commands

Once `Cargo.toml` exists:

```bash
cargo build
cargo test
cargo test <test_name>          # run a single test
cargo run --example hospital    # run the bundled hospital example
cargo clippy -- -D warnings
```

## Source layout

```
simu/
├── Cargo.toml
├── src/
│   ├── lib.rs           # public re-exports only
│   ├── env.rs           # SimEnv
│   ├── executor/
│   │   ├── mod.rs       # event loop, drives future polling
│   │   ├── queue.rs     # min-heap event queue with tie-breaking sequence numbers
│   │   └── waker.rs     # custom Waker implementation
│   ├── timeout.rs       # Timeout future
│   ├── event.rs         # EventTrigger, EventAwaitable
│   ├── resource/
│   │   ├── mod.rs       # Resource, ResourceRequest, ResourceGuard
│   │   ├── priority.rs  # PriorityResource (post-MVP)
│   │   └── preemptive.rs # PreemptiveResource (post-MVP)
├── examples/
│   └── hospital.rs
└── tests/
    ├── timeout.rs
    ├── event.rs
    └── resource.rs
```

`executor/` is entirely `pub(crate)` — not user-facing.

## Architecture

### Core design: single-threaded custom async executor per `SimEnv`

- **No tokio/async-std**. The executor is hand-rolled, polling futures based on simulated time, not wall-clock time.
- `SimEnv` is `!Send + !Sync` — it lives entirely on one thread.
- Monte Carlo parallelism is achieved by spinning up independent `SimEnv` instances on OS threads (`std::thread` or `rayon`).
- DES is inherently sequential within a run, so single-threaded execution gives zero sync overhead.

### Key types

| Type | Role |
|------|------|
| `SimEnv` | Central coordinator: owns the event queue (min-heap), current time (`f64`), active processes, and seeded RNG |
| `Timeout` | Future that resolves after a simulated delay |
| `EventTrigger` / `EventAwaitable` | Paired handles for manual inter-process signalling |
| `Resource` | FIFO-queued pool of capacity-limited units; acquisition is RAII via `ResourceGuard` |

### Event queue invariant

Events are ordered by simulated time, then by insertion order (sequence number) for deterministic tie-breaking. Given the same seed and process logic, a simulation must produce identical results.

### Resource ownership

Resources are created outside `SimEnv` and shared across processes via `Arc<Resource>` — safe without `Mutex` because the executor is single-threaded.

### Error strategy

- Programming errors (wrong API use) → `panic!`
- Simulation errors → `Result`

## Dependencies

| Crate | Purpose |
|-------|---------|
| `rand` + `rand_distr` | Seeded RNG and distributions (required) |
| `rayon` | Optional Monte Carlo helper (feature flag `monte-carlo`) |

## MVP scope

See `SPEC.md §5` for the full feature checklist. Post-MVP types (`PriorityResource`, `PreemptiveResource`, `AnyOf`/`AllOf`, `ProcessHandle`, `Interrupt`) are designed but explicitly out of scope until the MVP is complete.

// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `simcore` is a tiny deterministic **discrete-event simulation** (DES)
//! kernel for building system simulators, inspired by Python's
//! [SimPy](https://simpy.readthedocs.io/) but built to be idiomatic Rust,
//! fast, and reproducible.
//!
//! Simulation processes are ordinary `async` blocks driven by a custom,
//! single-threaded executor over *simulated* time — there is no tokio/async-std
//! and no wall-clock waiting. Processes interact through timeouts, manual
//! events, and shared resources; the event queue is ordered by
//! `(time, insertion)` so a run is fully deterministic given the same seed and
//! logic.
//!
//! **New to SimCore? Start with the [`tutorial`] module** — five short chapters
//! modeled on SimPy's "SimPy in 10 minutes", every snippet a running doc-test.
//!
//! The crates.io package is named **`simcore-des`** (the name `simcore` was
//! taken) but the library target is `simcore`: depend on `simcore-des = "0.1"`
//! and write `use simcore::…` exactly as in the examples here.
//!
//! # Quick start
//!
//! ```
//! use simcore::{SimEnv, Resource};
//!
//! let mut env = SimEnv::with_seed(42);
//! let machine = Resource::new(1); // a pool of one unit, shared by cloning
//!
//! let h = env.handle();
//! let m = machine.clone();
//! env.spawn(async move {
//!     let _guard = m.request().await; // queue for the machine (FIFO)
//!     h.timeout(2.0).await;           // hold it for 2 simulated time units
//! }); // guard drops here → unit released
//!
//! env.run(); // drive the event loop until the queue drains
//! assert_eq!(env.now(), 2.0);
//! ```
//!
//! # Core types
//!
//! | Type | Role |
//! |------|------|
//! | [`SimEnv`] | Owns the event loop, current time, and the seeded RNG. Not `Clone`; one per thread. |
//! | [`EnvHandle`] | Cheap `Clone` handle passed into processes: `now` / `timeout` / `event` / `spawn` / `rng`. |
//! | [`Timeout`] | Future resolving after a simulated delay. |
//! | [`EventTrigger`] / [`EventAwaitable`] | Manual inter-process signalling (multi-waiter, fire-before-await latch). |
//! | [`Resource`] / [`ResourceGuard`] | FIFO capacity-limited pool; RAII release on guard drop. |
//! | [`PriorityResource`] | Priority-scheduled pool (lower number = higher priority; FIFO within a level). |
//! | [`PreemptiveResource`] / [`PreemptiveGuard`] | Priority pool whose in-use units can be preempted (cooperative-at-yield). |
//! | [`Container`] | Reservoir of continuous quantity (`put` / `get`, strict head-of-line FIFO). |
//! | [`ProcessHandle`] | Observable spawn: `await` for the return value, drop to detach. |
//! | [`AnyOf`] / [`AllOf`] | Future combinators, built via the [`any_of!`] / [`all_of!`] macros. |
//!
//! # Threading and Monte Carlo
//!
//! [`SimEnv`] (and the resource handles) are `!Send + !Sync` — a simulation
//! lives entirely on one thread, which is why the executor needs no locking.
//! Parallelism comes from running *independent* simulations across threads:
//! [`monte_carlo::run`] executes a closure once per seed and returns the results
//! in seed order (one `std::thread` per seed by default; enable the
//! `monte-carlo` feature for a rayon-backed pool).
//!
//! # Randomness
//!
//! By default a [`SimEnv`] draws from `rand`'s `StdRng`. For cross-language
//! reproducibility, plug in a [`RandomSource`] via [`SimEnv::with_source`] — e.g.
//! the portable [`SplitMix64`] feed, whose stream and the [`rng::sample`]
//! transforms are mirrored in Python for exact SimPy comparison.
//!
//! # Feature flags
//!
//! | Feature | Default | Effect |
//! |---------|---------|--------|
//! | `monte-carlo` | off | Switches [`monte_carlo::run`] from one-`std::thread`-per-seed to rayon's bounded work-stealing pool — preferable for hundreds or thousands of seeds. |
//!
//! # Examples
//!
//! Four beginner examples (`intro_car`, `intro_charging`, `intro_cancellation`,
//! `intro_charging_station`) accompany the [`tutorial`] chapters. Three
//! end-to-end models combine everything: `hospital` (priority triage +
//! bed eviction + blood-bank `Container`), `brewery` (a bio-reactor production
//! line), and `warehouse` (a forklift fleet exercising [`PreemptiveResource`]).

#![warn(missing_docs)]

mod combinator;
mod env;
mod event;
pub mod monte_carlo;
mod process;
mod resource;
pub mod rng;
mod timeout;
pub mod tutorial;

pub use combinator::{AllOf, AnyOf};
pub use env::{EnvHandle, SimEnv};
pub use event::{EventAwaitable, EventTrigger};
pub use process::ProcessHandle;
pub use resource::{Container, ContainerGetRequest, ContainerPutRequest};
pub use resource::{PreemptiveGuard, PreemptiveRequest, PreemptiveResource};
pub use resource::{PriorityResource, PriorityResourceGuard, PriorityResourceRequest};
pub use resource::{Resource, ResourceGuard, ResourceRequest};
pub use rng::{RandomSource, SplitMix64};
pub use timeout::Timeout;

mod executor;

// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! **simu in 10 minutes** — a guided tour, one concept per chapter.
//!
//! This tutorial mirrors the structure of SimPy's
//! ["SimPy in 10 minutes"](https://simpy.readthedocs.io/en/latest/simpy_intro/)
//! so that readers who know SimPy can map their knowledge directly, and
//! newcomers get the gentlest possible on-ramp. Every code block is a doc-test:
//! it compiles and runs on every `cargo test`, so the tutorial cannot drift out
//! of date.
//!
//! | Chapter | You will learn |
//! |---------|----------------|
//! | [`ch01_basic_concepts`] | What a process is; spawning; timeouts; running the clock |
//! | [`ch02_waiting_for_processes`] | One process waiting for another to finish |
//! | [`ch03_events_and_cancellation`] | Signalling between processes; cancelling work early |
//! | [`ch04_shared_resources`] | Queuing for limited resources |
//! | [`ch05_how_to_proceed`] | The rest of the toolbox, and where to go next |
//!
//! Each chapter's example also exists as a runnable program in `examples/`
//! (`cargo run --example intro_car`, etc.).
//!
//! This module contains no code — only documentation.

/// Chapter 1: Basic concepts — processes, timeouts, and the clock.
///
/// A discrete-event simulation models a system as **processes** that do
/// something, wait, and do something again. While a process waits, simulated
/// time jumps directly to the next interesting moment — nothing "runs" in
/// between, which is why a simulated year can take milliseconds of wall-clock
/// time.
///
/// In simu, a process is an ordinary `async` block. Waiting is `.await`ing a
/// [`Timeout`](crate::Timeout): the executor suspends the process and resumes
/// it when the simulated clock reaches the deadline. There is no tokio and no
/// threads — one [`SimEnv`](crate::SimEnv) owns the clock and drives
/// everything.
///
/// Our first process models a car that alternately parks and drives:
///
/// ```
/// use simu::SimEnv;
///
/// let mut env = SimEnv::with_seed(42);
/// let h = env.handle(); // cheap Clone handle, moved into the process
///
/// env.spawn(async move {
///     loop {
///         println!("Start parking at {}", h.now());
///         h.timeout(5.0).await; // park for 5 time units
///
///         println!("Start driving at {}", h.now());
///         h.timeout(2.0).await; // drive for 2 time units
///     }
/// });
///
/// env.run_until(15.0); // drive the event loop until t = 15
/// assert_eq!(env.now(), 15.0);
/// ```
///
/// Output:
///
/// ```text
/// Start parking at 0
/// Start driving at 5
/// Start parking at 7
/// Start driving at 12
/// Start parking at 14
/// ```
///
/// Things worth noticing:
///
/// - [`SimEnv::with_seed`](crate::SimEnv::with_seed) makes the run
///   reproducible; same seed + same logic = identical results, always.
/// - The process gets an [`EnvHandle`](crate::EnvHandle) (`env.handle()`),
///   not the env itself: the env stays outside driving the loop, the handle
///   goes inside for `now()` / `timeout()` / `spawn()`.
/// - The process loops forever; that is fine.
///   [`run_until`](crate::SimEnv::run_until) stops the world at t = 15, and
///   dropping the env reclaims the still-suspended process.
/// - Time is `f64` and unit-less — *you* decide whether 1.0 means a second or
///   a day.
pub mod ch01_basic_concepts {}

/// Chapter 2: Waiting for another process.
///
/// Processes can start other processes and wait for them — the building block
/// for "do this sub-task, then continue". In SimPy you `yield env.process(...)`;
/// in simu, [`spawn`](crate::EnvHandle::spawn) returns a
/// [`ProcessHandle`](crate::ProcessHandle) which is itself a future: awaiting
/// it suspends you until the child process finishes and hands you its return
/// value.
///
/// Our car is now electric. After every trip it must charge before it can
/// drive again — and charging is its own process:
///
/// ```
/// use simu::SimEnv;
///
/// let mut env = SimEnv::with_seed(42);
/// let h = env.handle();
///
/// env.spawn(async move {
///     loop {
///         println!("Start driving at {}", h.now());
///         h.timeout(2.0).await;
///
///         println!("Start charging at {}", h.now());
///         let hc = h.clone();
///         let charging = h.spawn(async move {
///             hc.timeout(5.0).await;
///             42.0 // a process can return a value, e.g. the kWh charged
///         });
///         let kwh = charging.await; // suspend until charging finishes
///         assert_eq!(kwh, 42.0);
///     }
/// });
///
/// env.run_until(15.0);
/// ```
///
/// Output:
///
/// ```text
/// Start driving at 0
/// Start charging at 2
/// Start driving at 7
/// Start charging at 9
/// Start driving at 14
/// ```
///
/// Two details:
///
/// - Handles are cheap clones sharing one env; clone freely
///   (`let hc = h.clone()`) whenever a child process needs its own.
/// - If you *don't* need the result, just drop the
///   [`ProcessHandle`](crate::ProcessHandle) — the child keeps running,
///   fire-and-forget.
pub mod ch02_waiting_for_processes {}

/// Chapter 3: Events, and cancelling work early.
///
/// Timeouts model *known* waiting times. For "wait until something happens",
/// simu has manual events: [`env.event()`](crate::SimEnv::event) returns a
/// paired ([`EventTrigger`](crate::EventTrigger),
/// [`EventAwaitable`](crate::EventAwaitable)). Awaiting the awaitable suspends
/// a process until someone calls [`fire()`](crate::EventTrigger::fire) on the
/// trigger. The awaitable is `Clone`, so many processes can wait on one event;
/// firing after the fact is fine too — late awaiters resolve immediately.
///
/// SimPy's version of "stop what you're doing" is throwing an `Interrupt`
/// into a process. simu has no interrupt (it is on the roadmap — SPEC §6);
/// instead, cancellation is expressed by **racing futures** with
/// [`any_of!`](crate::any_of): await *either* the work finishing *or* a stop
/// signal, whichever comes first. The losing future is dropped — and dropping
/// *is* cancellation in Rust.
///
/// The driver gets impatient and stops a 5-unit charge after 3 units:
///
/// ```
/// use simu::{SimEnv, any_of};
///
/// let mut env = SimEnv::with_seed(42);
/// let (stop_charging, stop_signal) = env.event();
///
/// // The car: charge fully — unless told to stop.
/// let h = env.handle();
/// let car = env.spawn(async move {
///     println!("Start charging at {}", h.now());
///     any_of![h.timeout(5.0), stop_signal].await;
///     println!("Stop charging at {}", h.now());
///     h.now() // return when charging actually ended
/// });
///
/// // The driver: after 3 time units, wants to leave.
/// let h2 = env.handle();
/// env.spawn(async move {
///     h2.timeout(3.0).await;
///     stop_charging.fire(); // wake everyone awaiting the signal
/// });
///
/// let h3 = env.handle();
/// env.spawn(async move {
///     let stopped_at = car.await;
///     assert_eq!(stopped_at, 3.0); // the event won the race, not the timeout
///     let _ = h3; // (nothing else to do)
/// });
///
/// env.run();
/// ```
///
/// Notes:
///
/// - `fire()` **consumes** the trigger — an event fires at most once. A
///   dropped, never-fired trigger simply means the signal never arrives
///   (waiters stay suspended until the run ends), which is a normal
///   discrete-event outcome, not an error.
/// - After the race, the abandoned `timeout(5.0)` still has a queue entry;
///   its wakeup at t = 5 is a benign no-op. That is why `env.run()` above
///   ends at t = 5, not t = 3 — use `run_until` if the end time matters.
/// - For being kicked off a *resource* by higher-priority work, see
///   [`PreemptiveResource`](crate::PreemptiveResource) — same racing pattern,
///   built in.
pub mod ch03_events_and_cancellation {}

/// Chapter 4: Shared resources — queuing for limited capacity.
///
/// Real systems have contention: two charging spots, one doctor, three beds.
/// A [`Resource`](crate::Resource) models a pool of identical units.
/// [`request()`](crate::Resource::request) resolves immediately if a unit is
/// free, otherwise the process suspends in a FIFO queue. The resolved value is
/// an RAII [`ResourceGuard`](crate::ResourceGuard): the unit is released when
/// the guard drops — no explicit `release()` call, and no way to forget it.
///
/// Sharing works by **cloning the handle** — every clone is the same pool.
/// (No `Arc`, no `Mutex`: the whole simulation is single-threaded by design.)
///
/// Four cars arrive, staggered, at a two-spot battery charging station:
///
/// ```
/// use simu::{SimEnv, Resource};
///
/// let mut env = SimEnv::with_seed(42);
/// let bcs = Resource::new(2); // battery charging station, 2 spots
///
/// for i in 0..4u32 {
///     let h = env.handle();
///     let station = bcs.clone(); // same pool, cheap Rc clone
///     env.spawn(async move {
///         h.timeout(f64::from(i) * 2.0).await; // drive to the station
///         println!("Car {i} arriving at {}", h.now());
///
///         let _spot = station.request().await; // queue for a spot (FIFO)
///         println!("Car {i} starting to charge at {}", h.now());
///
///         h.timeout(5.0).await; // charge
///         println!("Car {i} leaving at {}", h.now());
///     }); // _spot drops here → spot handed to the next car in line
/// }
///
/// env.run();
/// assert_eq!(env.now(), 12.0); // last car: arrives t=6, waits, charges 7→12
/// ```
///
/// Output:
///
/// ```text
/// Car 0 arriving at 0
/// Car 0 starting to charge at 0
/// Car 1 arriving at 2
/// Car 1 starting to charge at 2
/// Car 2 arriving at 4
/// Car 0 leaving at 5
/// Car 2 starting to charge at 5
/// Car 3 arriving at 6
/// Car 1 leaving at 7
/// Car 3 starting to charge at 7
/// Car 2 leaving at 10
/// Car 3 leaving at 12
/// ```
///
/// Cars 0 and 1 charge immediately; cars 2 and 3 queue and take over spots the
/// moment earlier cars leave. Release-to-next-waiter is a **direct handoff**:
/// a freshly released unit can never be stolen by a same-instant new request
/// jumping the queue.
pub mod ch04_shared_resources {}

/// Chapter 5: How to proceed.
///
/// You now know the core loop of every simu model: spawn processes, await
/// timeouts / events / resources, run, read out results. The rest of the
/// toolbox, in the order you are likely to need it:
///
/// - [`PriorityResource`](crate::PriorityResource) — like
///   [`Resource`](crate::Resource), but `request(priority)` serves lower
///   numbers first (FIFO within a level). Triage queues, VIP lanes.
/// - [`PreemptiveResource`](crate::PreemptiveResource) — a priority pool where
///   an urgent request can *evict* a lower-priority holder mid-service; the
///   victim observes it via `guard.preempted()`. See the type docs for the
///   full pattern.
/// - [`Container`](crate::Container) — continuous quantity instead of discrete
///   units: tanks, silos, blood banks. `put(amount)` / `get(amount)` with
///   strict FIFO waiters.
/// - [`all_of!`](crate::all_of) — the dual of
///   [`any_of!`](crate::any_of): wait for *every* sub-future (barrier /
///   fork-join).
/// - **Randomness** — [`h.rng()`](crate::EnvHandle::rng) borrows the env's
///   seeded RNG; combine with [`rng::sample`](crate::rng::sample) or
///   `rand_distr` for stochastic arrival/service times. Sample *before*
///   `.await` — the borrow cannot be held across a suspension point.
/// - **Monte Carlo** — [`monte_carlo::run`](crate::monte_carlo::run) executes
///   one full, independent simulation per seed in parallel threads:
///
/// ```
/// use simu::{SimEnv, monte_carlo};
///
/// let end_times = monte_carlo::run(0..8u64, |seed| {
///     let mut env = SimEnv::with_seed(seed);
///     let h = env.handle();
///     env.spawn(async move { h.timeout(1.0).await; });
///     env.run();
///     env.now()
/// });
/// assert_eq!(end_times.len(), 8); // results arrive in seed order
/// ```
///
/// When you are ready for full models, three commented showcases combine
/// everything above, each with a walkthrough document in `examples/`:
///
/// - `cargo run --example hospital` — ER with priority triage, bed eviction,
///   and a blood bank (`PriorityResource`, `PreemptiveResource` precursor
///   patterns, `Container`).
/// - `cargo run --example brewery` — a fermentation line with contamination
///   events and cleanup priorities (`EventTrigger`, `PriorityResource`).
/// - `cargo run --example warehouse` — a forklift fleet shared between
///   receiving and shipping (`PreemptiveResource` end-to-end).
///
/// Coming from SimPy? The repository root has `llms.txt` with a complete
/// SimPy → simu translation table.
pub mod ch05_how_to_proceed {}

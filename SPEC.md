# simu — Discrete Event Simulation Library Specification

## 1. Project Overview

`sim-rs` is a Rust library for Discrete Event Simulation (DES), inspired by Python's SimPy but designed
from the ground up to be idiomatic Rust, high-performance, and scalable. The primary target use case is
complex workflow simulation (e.g., hospital operations), where thousands of independent processes
interact through shared resources and events.

---

## 2. Crate Naming

The library crate name is `simu`: short, implies simulation, and likely unclaimed on crates.io.

---

## 3. Non-Functional Requirements

| Property            | Requirement                                                             |
|---------------------|-------------------------------------------------------------------------|
| **Performance**     | Must support thousands of concurrent processes with low overhead        |
| **Scalability**     | Monte Carlo parallelism via OS threads (`std::thread`)                  |
| **Determinism**     | Given the same seed and configuration, a simulation must be reproducible |
| **Correctness**     | Events at equal simulation time must be processed in deterministic order |
| **Idiomatic Rust**  | Public API uses standard Rust patterns; no unsafe in user-facing code   |
| **Error strategy**  | Programming errors (wrong API use) may panic; simulation errors use `Result` |

---

## 4. Architecture

### 4.1 Core Execution Model

DES is inherently sequential within a single simulation run: the scheduler processes one event at a
time, advancing simulated time monotonically. This means real parallelism within a single run provides
no benefit — and would introduce synchronisation overhead.

**Chosen approach: single-threaded custom async executor per simulation instance.**

- Each simulation process is an `async fn`.
- A custom executor (not tokio/async-std) drives process execution based on simulated time, not
  wall-clock time.
- The executor polls futures manually; no OS threads or I/O reactors are involved per simulation.
- Monte Carlo parallelism is achieved by running independent simulation instances on separate OS threads.

This gives maximum per-simulation throughput (zero sync overhead) while enabling multi-core utilisation
across runs.

```
┌─────────────────────────────────────────────────────────┐
│  Monte Carlo Driver (rayon / std::thread)                │
│                                                          │
│  Thread 0            Thread 1            Thread N        │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐ │
│  │  SimEnv      │   │  SimEnv      │   │  SimEnv      │ │
│  │  EventQueue  │   │  EventQueue  │   │  EventQueue  │ │
│  │  Processes   │   │  Processes   │   │  Processes   │ │
│  └──────────────┘   └──────────────┘   └──────────────┘ │
└─────────────────────────────────────────────────────────┘
```

### 4.2 Simulation Environment (`SimEnv`)

The environment is the central coordinator for a single simulation run. It owns:

- The **event queue** (min-heap ordered by simulated time, then by insertion order for tie-breaking).
- The **current simulation time** (`f64`).
- The set of **active processes** (futures awaiting simulation events).
- A **seeded RNG** for reproducible random sampling.

`SimEnv` is not `Send` or `Sync`. It is designed to live entirely on one thread.

```rust
pub struct SimEnv { /* opaque */ }

impl SimEnv {
    /// Create a new environment starting at time 0.0.
    pub fn new() -> Self;

    /// Create with a specific RNG seed for reproducibility.
    pub fn with_seed(seed: u64) -> Self;

    /// Current simulation time.
    pub fn now(&self) -> f64;

    /// Spawn a new process into the simulation.
    pub fn spawn<F>(&self, process: F)
    where
        F: Future<Output = ()> + 'static;

    /// Run the simulation until the event queue is empty.
    pub fn run(&mut self);

    /// Run until simulated time reaches `until`.
    pub fn run_until(&mut self, until: f64);

    /// Create a timeout event that resolves after `delay` simulated time units.
    pub fn timeout(&self, delay: f64) -> Timeout;

    /// Create an event that can be triggered externally by another process.
    pub fn event(&self) -> (EventTrigger, EventAwaitable);
}
```

### 4.3 Process Model

A process is any `async fn` (or `async` block) that accepts a reference or handle to the environment.
Processes interact with the simulation by awaiting simulation primitives.

```rust
async fn patient_journey(env: EnvHandle, resources: HospitalResources) {
    // Request a bed (blocks if none available)
    let _bed = resources.beds.request().await;

    // Wait 15 simulated time units for triage
    env.timeout(15.0).await;

    // Request a doctor
    let _doctor = resources.doctors.request().await;

    // Treatment duration sampled from distribution
    let duration = resources.rng().sample(Exp::new(1.0 / 45.0).unwrap());
    env.timeout(duration).await;

    // Resources released automatically when guards are dropped
}
```

Key design choices:
- Processes are spawned with `env.spawn(future)` and run lazily by the scheduler.
- Process handles (`ProcessHandle`) allow one process to wait for another to finish (post-MVP).
- Panicking inside a process terminates that process and propagates as a simulation error.

### 4.4 Event Model (MVP)

Two event types are supported in the MVP:

| Event type      | Description                                                     |
|-----------------|-----------------------------------------------------------------|
| `Timeout`       | Resolves when sim time advances by `delay` units                |
| `EventAwaitable`| Resolves when explicitly triggered via its paired `EventTrigger` |

Both implement `Future<Output = ()>` and can be directly `.await`ed inside a process.

```rust
// Timeout
env.timeout(10.0).await;

// Manual event (e.g., a signal between processes)
let (trigger, awaitable) = env.event();
env.spawn(async move {
    env.timeout(5.0).await;
    trigger.fire();  // wake the waiter at sim time 5
});
awaitable.await;
```

**Post-MVP additions:** `AnyOf`, `AllOf` combinators; `Interrupt` (preemption); `Condition`.

### 4.5 Resource Model (MVP)

A `Resource` models a pool of identical, limited-capacity units (e.g., hospital beds).

```rust
pub struct Resource { /* opaque */ }

impl Resource {
    pub fn new(capacity: usize) -> Self;

    /// Request one unit. Suspends the calling process if none are available (FIFO).
    pub fn request(&self) -> ResourceRequest;

    /// Current number of units in use.
    pub fn in_use(&self) -> usize;

    /// Total capacity.
    pub fn capacity(&self) -> usize;
}
```

Acquisition is RAII: the returned `ResourceGuard` releases the unit when dropped.

```rust
let guard = resource.request().await;  // waits if at capacity
// use resource ...
drop(guard);  // unit is released; next waiter is woken
```

Resource ownership: resources are typically created outside the environment and passed into processes
via `Arc` (since multiple processes share them, but still within one thread — `Arc<Resource>` without
`Mutex` is safe given the single-threaded executor).

**Post-MVP resource types:**

| Type                  | Priority |
|-----------------------|----------|
| `PriorityResource`    | High     |
| `PreemptiveResource`  | High     |
| `Container`           | Low      |
| `Store` / `FilterStore` | Low    |

### 4.6 Monte Carlo Parallelism

Each simulation run is a pure function of its inputs (config + seed). Multiple runs are launched on
OS threads. The recommended pattern:

```rust
use std::thread;

let handles: Vec<_> = configs
    .into_iter()
    .enumerate()
    .map(|(seed, config)| {
        thread::spawn(move || run_simulation(seed as u64, config))
    })
    .collect();

let results: Vec<SimResult> = handles.into_iter().map(|h| h.join().unwrap()).collect();
```

Alternatively, `rayon::iter::IntoParallelIterator` can be used for automatic thread pool management.
Both patterns are supported; `sim-rs` imposes no constraints on how the caller parallelises runs.

**Requirement:** `SimEnv` must be constructible from a seed and must produce identical event sequences
given the same seed and process logic.

---

## 5. MVP Feature Set

| Feature                            | Status |
|------------------------------------|--------|
| `SimEnv` with event queue          | MVP    |
| `Timeout` event                    | MVP    |
| Manual `Event` (trigger/await)     | MVP    |
| `Resource` with FIFO queue         | MVP    |
| RAII `ResourceGuard`               | MVP    |
| `env.spawn(async_fn)`              | MVP    |
| `env.run()` / `env.run_until()`    | MVP    |
| Seeded RNG (via `rand` crate)      | MVP    |
| Deterministic tie-breaking         | MVP    |
| Monte Carlo via `std::thread`      | MVP    |
| Hospital example (see §7)          | MVP    |
| Unit tests for all core primitives | MVP    |

---

## 6. Post-MVP Roadmap

Listed in priority order:

1. **`PriorityResource`** — request with priority level; higher priority jumps the queue.
2. **`PreemptiveResource`** — higher-priority request can preempt a current holder.
3. **`AnyOf` / `AllOf` combinators** — wait for the first/all of a set of events.
4. **`ProcessHandle`** — await the completion of a spawned process.
5. **`Interrupt`** — one process can interrupt another (e.g., emergency preemption).
6. **Event recording and replay** — log all events with timestamps; replay for deterministic debugging
   and regression testing.
7. **`RealtimeEnvironment`** — synchronise simulated time to wall-clock time (for training/demos).
8. **`Container`** — continuous-quantity resource (e.g., blood supply in litres).
9. **`Store` / `FilterStore`** — discrete-item queues with optional filter predicate.
10. **GPU/CUDA acceleration** — batch evaluation of independent sub-simulations on GPU. Applicable
    only when process logic can be expressed as data-parallel kernels (e.g., pure queuing networks).
    Requires further design work; depends on CUDA Rust bindings maturity.

---

## 7. Example: Minimalistic Hospital Simulation

The bundled example (`examples/hospital.rs`) models:

| Entity         | Count | Type            |
|----------------|-------|-----------------|
| Beds           | 3     | `Resource`      |
| Doctors        | 3     | `Resource`      |
| Nurses         | 5     | `Resource`      |
| CT scanner     | 1     | `Resource`      |
| Ultrasound     | 3     | `Resource`      |
| Live monitors  | 3     | `Resource`      |
| Pharmacy       | 1     | `Resource`      |
| Blood analyzer | 1     | `Resource`      |

**Patient flow:**

1. Patient arrives (Poisson inter-arrival times).
2. Requests a **bed** (waits if full → ER backlog).
3. Requests a **nurse** for initial assessment (timeout ~ 10 min).
4. Optionally requests **CT** or **ultrasound** for diagnosis (timeout ~ 20–40 min).
5. Requests a **doctor** for treatment decision (timeout ~ 15–30 min).
6. Requests **pharmacy** for medication (timeout ~ 5 min).
7. Optionally requests **blood analyzer** (timeout ~ 10 min).
8. Recovery on **live monitor** if critical (timeout ~ 60–120 min).
9. Releases all resources; patient discharged.

The example runs 10 parallel Monte Carlo simulations with different seeds, collects summary statistics
(mean wait time per resource, throughput), and prints a comparison table.

---

## 8. Dependency Plan

| Crate         | Purpose                                  | Feature flag |
|---------------|------------------------------------------|--------------|
| `rand`        | Seeded RNG, distributions                | required     |
| `rand_distr`  | Exponential, Normal, Poisson, etc.       | required     |
| `rayon`       | Optional parallel Monte Carlo helper     | `monte-carlo`|
| `thiserror`   | Error types                              | required     |

No async runtime dependency (tokio, async-std) — the custom executor is self-contained.

---

## 9. Out of Scope for MVP

- Real-time synchronisation.
- Networked / distributed simulation.
- GUI or visualisation.
- Monitoring / statistics collection (left to the application layer).
- GPU acceleration.
- Event recording and replay.
- Process interrupts and preemption.
- `Container`, `Store`, `FilterStore` resource types.
- `AnyOf` / `AllOf` event combinators.

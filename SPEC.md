# simu — Discrete Event Simulation Library Specification

## 1. Project Overview

`simu` is a Rust library for Discrete Event Simulation (DES), inspired by Python's SimPy but designed
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

- Each simulation process is an `async fn` or `async` block.
- A custom executor (not tokio/async-std) drives process execution based on simulated time, not
  wall-clock time.
- The executor polls futures manually; no OS threads or I/O reactors are involved per simulation.
- Monte Carlo parallelism is achieved by running independent simulation instances on separate OS threads.

This gives maximum per-simulation throughput (zero sync overhead) while enabling multi-core utilisation
across runs.

```
┌─────────────────────────────────────────────────────────┐
│  Monte Carlo Driver (monte_carlo::run / std::thread)     │
│                                                          │
│  Thread 0            Thread 1            Thread N        │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐ │
│  │  SimEnv      │   │  SimEnv      │   │  SimEnv      │ │
│  │  EventQueue  │   │  EventQueue  │   │  EventQueue  │ │
│  │  Processes   │   │  Processes   │   │  Processes   │ │
│  └──────────────┘   └──────────────┘   └──────────────┘ │
└─────────────────────────────────────────────────────────┘
```

### 4.2 Simulation Environment (`SimEnv` and `EnvHandle`)

The environment is split into two types:

- **`SimEnv`** — owns all simulation state and drives the event loop. Not cloneable; lives on one thread.
- **`EnvHandle`** — a lightweight, `Clone`able handle that processes use to interact with the simulation.
  Obtained via `env.handle()` and passed into spawned processes.

Both share the same underlying `SimState` and `StdRng` via `Rc<RefCell<>>`.

`SimEnv` is `!Send + !Sync` (via `Rc`) and must live on one thread.

```rust
pub struct SimEnv { /* opaque */ }

impl SimEnv {
    /// Create a new environment seeded from OS entropy.
    pub fn new() -> Self;

    /// Create with a specific RNG seed for reproducibility.
    pub fn with_seed(seed: u64) -> Self;

    /// Return a cloneable handle for passing into processes.
    pub fn handle(&self) -> EnvHandle;

    /// Current simulation time.
    pub fn now(&self) -> f64;

    /// Spawn a new process into the simulation.
    pub fn spawn<F: Future<Output = ()> + 'static>(&self, process: F);

    /// Run until the event queue is empty.
    pub fn run(&mut self);

    /// Run until simulated time reaches `until`.
    pub fn run_until(&mut self, until: f64);

    /// Create a timeout event. Convenience wrapper around `EnvHandle::timeout`.
    pub fn timeout(&self, delay: f64) -> Timeout;

    /// Create a paired event handle. Convenience wrapper around `EnvHandle::event`.
    pub fn event(&self) -> (EventTrigger, EventAwaitable);
}

#[derive(Clone)]
pub struct EnvHandle { /* opaque */ }

impl EnvHandle {
    /// Current simulation time.
    pub fn now(&self) -> f64;

    /// Create a `Timeout` that resolves after `delay` simulated time units.
    pub fn timeout(&self, delay: f64) -> Timeout;

    /// Create a paired `(EventTrigger, EventAwaitable)` for inter-process signalling.
    pub fn event(&self) -> (EventTrigger, EventAwaitable);

    /// Spawn a child process from within a running process.
    pub fn spawn<F: Future<Output = ()> + 'static>(&self, future: F);

    /// Borrow the shared RNG. The returned guard implements `RngCore`.
    /// Must not be held across an `.await` point.
    pub fn rng(&self) -> impl RngCore + '_;
}
```

### 4.3 Process Model

A process is any `async fn` or `async` block that receives an `EnvHandle`. Processes interact with the
simulation by awaiting simulation primitives.

```rust
async fn patient_journey(env: EnvHandle, resources: HospitalResources) {
    // Request a bed (blocks if none available)
    let _bed = resources.beds.request().await;

    // Wait 15 simulated time units for triage
    env.timeout(15.0).await;

    // Request a doctor
    let _doctor = resources.doctors.request().await;

    // Treatment duration sampled from distribution.
    // RNG must be sampled before the .await — the guard cannot cross an await point.
    let duration = env.rng().sample(Exp::new(1.0 / 45.0).unwrap());
    env.timeout(duration).await;

    // Resources released automatically when guards are dropped
}
```

Key design choices:
- Processes are spawned with `env.spawn(future)` and run lazily by the scheduler.
- `EnvHandle` is `Clone` — processes clone it rather than borrowing.
- Panicking inside a process terminates that process and propagates as a simulation error.
- Process handles (`ProcessHandle`) allowing one process to join another are post-MVP.

### 4.4 Event Model (MVP)

Two event types are supported in the MVP:

| Event type       | Description                                                      |
|------------------|------------------------------------------------------------------|
| `Timeout`        | Resolves when sim time advances by `delay` units                 |
| `EventAwaitable` | Resolves when explicitly triggered via its paired `EventTrigger` |

Both implement `Future<Output = ()>` and can be directly `.await`ed inside a process.

```rust
// Timeout
env.timeout(10.0).await;

// Manual event (e.g., a signal between processes)
let (trigger, awaitable) = env.event();
env.spawn(async move {
    env.timeout(5.0).await;
    trigger.fire();  // consumes trigger; wakes all current and future waiters
});
awaitable.await;
```

**Multi-waiter support:** `EventAwaitable` is `Clone`. Multiple processes can await the same event;
all are woken when `trigger.fire()` is called.

**Fire-before-await latch:** If `trigger.fire()` is called before any process awaits the event, the
`fired` flag is set. Any subsequent `.await` on the awaitable resolves immediately without suspending.

**`EventTrigger::fire` consumes `self`** — a trigger can only be fired once.

**Combinators** `AnyOf` and `AllOf` compose any `Future<Output = ()>` futures:

```rust
use simu::{any_of, all_of};

// Race: resolve when the first of several events fires
any_of![h.timeout(10.0), signal.clone()].await;

// Barrier: resolve when all events have fired
all_of![phase_a, phase_b, phase_c].await;
```

Both accept one or more expressions via macro (which auto-`Box::pin` each);
`AnyOf::new(vec![])` panics, `AllOf::new(vec![])` resolves immediately.

**Post-MVP additions:** `Interrupt` (preemption); `Condition`.

### 4.5 Resource Model (MVP)

A `Resource` models a pool of identical, limited-capacity units (e.g., hospital beds).

```rust
pub struct Resource { /* opaque, Clone */ }

impl Resource {
    /// # Panics
    /// Panics if `capacity` is zero.
    pub fn new(capacity: usize) -> Self;

    /// Request one unit. Suspends the calling process if none are available (FIFO).
    /// Returns a guard that releases the unit when dropped.
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
drop(guard);  // unit is released; next waiter is woken (FIFO)
```

**Resource ownership:** `Resource` wraps `Rc<RefCell<ResourceState>>` internally and implements
`Clone`. All clones share the same pool. There is no need for `Arc` or `Mutex` because the executor
is single-threaded. Resources are created outside `SimEnv` and shared across processes by cloning:

```rust
let machine = Resource::new(1);

for _ in 0..3 {
    let m = machine.clone();   // cheap Rc clone
    env.spawn(async move {
        let _guard = m.request().await;
        // ...
    });
}
```

`Resource` is `!Send + !Sync` — consistent with `SimEnv`.

**`PriorityResource`** is also available when priority scheduling is needed:

```rust
pub struct PriorityResource { /* Clone, !Send+!Sync */ }

impl PriorityResource {
    pub fn new(capacity: usize) -> Self;

    /// Request one unit. Lower priority number = higher priority (0 is highest).
    /// Within the same priority level, requests are served FIFO.
    pub fn request(&self, priority: u32) -> PriorityResourceRequest;

    pub fn in_use(&self) -> usize;
    pub fn capacity(&self) -> usize;
}
```

```rust
let nurse = PriorityResource::new(1);
// critical patients (priority 0) jump ahead of standard patients (priority 1)
let _guard = nurse.request(triage_level).await;
```

**Post-MVP resource types:**

| Type                  | Status   |
|-----------------------|----------|
| `PreemptiveResource`  | Post-MVP |
| `Container`           | Post-MVP |
| `Store` / `FilterStore` | Post-MVP |

### 4.6 Monte Carlo Parallelism

Each simulation run is a pure function of its inputs (config + seed). Multiple runs are launched on
OS threads. The recommended pattern uses the built-in `monte_carlo::run` helper:

```rust
use simu::monte_carlo;

let results = monte_carlo::run(0..10, |seed| {
    let mut env = SimEnv::with_seed(seed);
    // ... build and run simulation ...
    env.run();
    env.now()
});
// results[i] corresponds to seed i
```

`monte_carlo::run` wraps the closure in an `Arc`, spawns one `std::thread` per seed, and collects
results in seed order. Because `SimEnv` is created *inside* each closure, it never crosses thread
boundaries and its `!Send` nature is not a problem.

For finer control, threads can be managed manually:

```rust
use std::thread;

let handles: Vec<_> = (0..10u64)
    .map(|seed| thread::spawn(move || run_simulation(seed)))
    .collect();

let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
```

**Requirement:** `SimEnv` must produce identical event sequences given the same seed and process logic.

---

## 5. MVP Feature Set

All MVP features are implemented.

| Feature                            | Status      |
|------------------------------------|-------------|
| `SimEnv` with event queue          | Done ✅     |
| `Timeout` event                    | Done ✅     |
| Manual `Event` (trigger/await)     | Done ✅     |
| Multi-waiter event support         | Done ✅     |
| Fire-before-await latch            | Done ✅     |
| `Resource` with FIFO queue         | Done ✅     |
| RAII `ResourceGuard`               | Done ✅     |
| `env.spawn(async_fn)`              | Done ✅     |
| `env.run()` / `env.run_until()`    | Done ✅     |
| Seeded RNG (via `rand` crate)      | Done ✅     |
| Deterministic tie-breaking         | Done ✅     |
| Monte Carlo via `monte_carlo::run` | Done ✅     |
| Hospital example (see §7)          | Done ✅     |
| `PriorityResource` with priority heap | Done ✅  |
| `AnyOf` / `AllOf` combinators      | Done ✅     |
| `any_of!` / `all_of!` macros       | Done ✅     |
| Integration test suite (39 tests)  | Done ✅     |
| Criterion benchmark suite          | Done ✅     |

---

## 6. Post-MVP Roadmap

Listed in priority order:

1. **`PreemptiveResource`** — higher-priority request can preempt a current holder.
2. **`ProcessHandle`** — await the completion of a spawned process.
3. **`Interrupt`** — one process can interrupt another (e.g., emergency preemption).
4. **Event recording and replay** — log all events with timestamps; replay for deterministic debugging
   and regression testing.
5. **`RealtimeEnvironment`** — synchronise simulated time to wall-clock time (for training/demos).
6. **`Container`** — continuous-quantity resource (e.g., blood supply in litres).
7. **`Store` / `FilterStore`** — discrete-item queues with optional filter predicate.
8. **GPU/CUDA acceleration** — batch evaluation of independent sub-simulations on GPU. Applicable
   only when process logic can be expressed as data-parallel kernels (e.g., pure queuing networks).
   Requires further design work; depends on CUDA Rust bindings maturity.

---

## 7. Example: Hospital Simulation

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

The example runs 10 parallel Monte Carlo simulations via `monte_carlo::run`. Each run writes output to
a dedicated `hospital_run_<N>.log` file. After all runs complete, a summary table of mean wait times
and patient throughput is printed to stdout.

---

## 8. Dependency Plan

| Crate         | Purpose                                  | Type            |
|---------------|------------------------------------------|-----------------|
| `rand`        | Seeded RNG, `RngCore` trait              | required        |
| `rand_distr`  | Exponential, Normal, Poisson, etc.       | required        |
| `rayon`       | Optional parallel Monte Carlo helper     | optional (`monte-carlo` feature) |
| `thiserror`   | Error types                              | required        |
| `criterion`   | Statistical benchmark harness            | dev-dependency  |

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

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
    pub fn spawn<F>(&self, process: F) -> ProcessHandle<F::Output>
    where F: Future + 'static, F::Output: 'static;

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
    pub fn spawn<F>(&self, future: F) -> ProcessHandle<F::Output>
    where F: Future + 'static, F::Output: 'static;

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
- `spawn` returns a [`ProcessHandle<T>`](#4-4-core-types) that resolves to the
  process's return value. Dropping the handle detaches the process
  (fire-and-forget). `ProcessHandle<T>` is not `Clone` — broadcast patterns
  use [`EventTrigger`](#event) instead.

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

#### ProcessHandle

`spawn` returns a `ProcessHandle<T>` that is itself a `Future<Output = T>`.
Awaiting the handle suspends the caller until the spawned process finishes,
and yields its return value. Handles are **not `Clone`** — single-await,
tokio-`JoinHandle`-style. Broadcast patterns should use `EventTrigger`.

```rust
pub struct ProcessHandle<T> { /* opaque, T: 'static */ }

impl<T: 'static> Future for ProcessHandle<T> {
    type Output = T;
}

impl<T: 'static> ProcessHandle<T> {
    /// Await and discard the value — for use with `any_of!` / `all_of!`.
    pub fn discard(self) -> impl Future<Output = ()> + 'static;
}
```

Dropping the handle before awaiting **detaches** the process: it keeps
running; its return value, if any, is dropped when the process completes.
This matches `tokio::JoinHandle` semantics.

Typical patterns:

```rust
// Return a value from a process
let h = env.spawn(async { env.timeout(10.0).await; compute_result() });
let result = h.await;

// Join multiple child processes as a barrier
let a = env.spawn(phase_a());
let b = env.spawn(phase_b());
all_of![a.discard(), b.discard()].await;

// Fire-and-forget (idiomatic — just drop the returned handle)
env.spawn(background_work());
```

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

**`Container`** models a reservoir of continuous quantity (e.g., blood supply, fuel):

```rust
pub struct Container { /* Clone, !Send+!Sync */ }

impl Container {
    /// Create empty container. Panics if capacity <= 0.
    pub fn empty(capacity: f64) -> Self;

    /// Create with initial level. Panics if capacity <= 0, initial_level < 0,
    /// or initial_level > capacity.
    pub fn new(capacity: f64, initial_level: f64) -> Self;

    pub fn level(&self) -> f64;
    pub fn capacity(&self) -> f64;

    /// Add `amount`. Suspends if level + amount > capacity. Panics if amount <= 0.
    pub fn put(&self, amount: f64) -> ContainerPutRequest;

    /// Remove `amount`. Suspends if level < amount. Panics if amount <= 0.
    pub fn get(&self, amount: f64) -> ContainerGetRequest;
}
```

Both `put` and `get` suspend when they cannot immediately complete. Waiters are
served **FIFO**. The level change is committed eagerly by the wake cascade (not
on re-poll), so processes always see the correct level after `.await`.

**Remaining post-MVP resource types:**

| Type                  | Status   |
|-----------------------|----------|
| `PreemptiveResource`  | Post-MVP |
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

If any worker thread panics, the original panic payload is re-raised on the
calling thread via `std::panic::resume_unwind` (after all siblings have been
joined, so no threads are orphaned).

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

## 4.7 Internal invariants

The following patterns are shared across all suspendable primitives. They are
implementation details but are documented because they are load-bearing for
correctness.

### `registered` flag

Every request-future (`ResourceRequest`, `PriorityResourceRequest`,
`ContainerGetRequest`, `ContainerPutRequest`) carries a `registered: bool`
flag. On first poll the future enqueues a waiter; on subsequent polls the
`registered` check prevents double-queuing. Once registered, the future only
returns `Ready` when the wake cascade explicitly marks it done — a
spurious re-poll (from an unrelated waker) cannot steal capacity ahead of an
earlier waiter and thereby violate FIFO ordering.

### `canceled` flag on waiter entries

If a registered request-future is dropped before being granted (for example,
a competing arm of `any_of!` resolves first), its `Drop` impl sets a shared
`Rc<Cell<bool>>` canceled flag on the queue entry. Guard-release loops
(`Resource`, `PriorityResource`) and the Container wake-cascade skip canceled
entries, preserving two invariants:

- **No waiter starvation**: a live waiter behind a dropped one is still woken.
- **No material leak in `Container`**: the cascade never deducts level for an
  abandoned `get`, nor adds level for an abandoned `put`.

### `trigger_cascade` for `Container`

After any level change (successful `put` or `get`), `trigger_cascade` loops
over `wake_get_waiters` and `wake_put_waiters` until the level stabilises.
One iteration is sufficient for typical workloads; the loop handles chains
where a put immediately enables a get, which immediately enables another
put, and so on, all within a single call.

### `RngGuard` and the no-await invariant

`EnvHandle::rng()` returns an `impl RngCore + '_` wrapper over a `RefMut` into
the SimEnv's RNG. Because `RefMut` is `!Send` and borrows `self`, the returned
guard cannot cross an `.await` point — the compiler rejects any such
misuse. This makes deterministic sampling safe by construction.

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
| `Container` (continuous quantity)  | Done ✅     |
| `ProcessHandle<T>` (observable spawn) | Done ✅  |
| Integration test suite (66 tests)  | Done ✅     |
| Criterion benchmark suite          | Done ✅     |

---

## 6. Post-MVP Roadmap

Listed in priority order:

1. **`PreemptiveResource`** — higher-priority request can preempt a current holder.
2. **`Interrupt`** — one process can interrupt another (e.g., emergency preemption).
3. **`RealtimeEnvironment`** — synchronise simulated time to wall-clock time (for training/demos).
4. **`Store` / `FilterStore`** — discrete-item queues with optional filter predicate.
5. **GPU/CUDA acceleration** — batch evaluation of independent sub-simulations on GPU. Applicable
   only when process logic can be expressed as data-parallel kernels (e.g., pure queuing networks).
   Requires further design work; depends on CUDA Rust bindings maturity.

---

## 7. Examples

Two end-to-end examples ship in `examples/`. Each runs 10 parallel Monte Carlo simulations via
`monte_carlo::run`, writes a per-run log file, and prints a summary table to stdout. The full
walkthroughs (configuration, sequence diagrams, sample output) live in `examples/hospital.md` and
`examples/brewery.md`.

### 7.1 Hospital Simulation

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

### 7.2 Brewery Simulation

The second bundled example (`examples/brewery.rs`) covers the **food & beverage / process
automation** domain: a craft brewery whose central bio-reactor (the fermenter) is the natural
bottleneck.

| Entity              | Count | Type                | Role                                       |
|---------------------|------:|---------------------|--------------------------------------------|
| Mash tuns           |     2 | `Resource`          | Hot-water mashing of grain                 |
| Kettles             |     2 | `Resource`          | Wort boiling + hop addition                |
| Fermenters          |     5 | `Resource`          | The bio-reactors (longest hold per batch)  |
| Conditioning tanks  |     4 | `Resource`          | Post-fermentation maturation               |
| Bottling line       |     1 | `PriorityResource`  | Premium batches (priority 0) preempt standard (priority 1) |
| CIP crew            |     1 | `PriorityResource`  | Urgent contamination CIP (priority 0) preempts routine (priority 1) |
| Hot-water buffer    |   one | `Container`         | Drawn during mashing, periodic restock     |
| Yeast slurry        |   one | `Container`         | Drawn at start of fermentation, periodic propagation |
| CO₂ recovery        |   one | `Container`         | Filled during boil                         |
| Bulk-beer storage   |   one | `Container`         | Filled by conditioning, drained by bottling |

**Batch flow:**

1. Order arrives (Poisson inter-arrival; 25% premium).
2. Acquires a **mash tun** and draws 500 L of **hot water** (timeout ~ 2 h).
3. Acquires a **kettle**, boils 1.5 h, returns CO₂ to recovery.
4. Draws 5 L of **yeast slurry**, requests a **fermenter**, runs `any_of![timeout(~60 h),
   contamination_signal]`.
5. If contamination wins → release fermenter, request **CIP crew** with priority 0
   (preempts routine cleanups), terminate. The batch is lost.
6. Otherwise → acquire **conditioning tank** (~ 12 h), put 800 L of beer into bulk storage.
7. Acquire **bottling line** with priority based on premium flag, draw 800 L from bulk
   storage, bottle (~ 6 h).
8. Acquire **CIP crew** with priority 1, run routine cleanup (~ 1 h), release.

**Contamination mechanism.** A separate `qa_inspector` process ticks at Poisson intervals and,
with probability `CONTAMINATION_PROB`, picks the oldest in-flight fermentation from a
`BTreeMap<batch_id, EventTrigger>` and fires its trigger. The fermenting batch's `any_of!`
resolves on the signal branch; comparing `env.now()` against the planned deadline tells the batch
whether contamination won.

**`AllOf` join.** The `arrivals` process collects every spawned `ProcessHandle<()>::discard()`
and, after the order book closes, awaits `AllOf` on the whole vector so the harness can record
the simulated time the line is fully drained.

Per-run logs are written to `brewery_run_<N>.log`. The summary table reports arrivals per class,
contaminated batches, total litres bottled, and mean wait times for the fermenter, bottling line,
and yeast pool.

---

## 8. Dependency Plan

| Crate         | Purpose                                  | Type            |
|---------------|------------------------------------------|-----------------|
| `rand`        | Seeded RNG, `RngCore` trait              | required        |
| `rand_distr`  | Exponential, Normal, Poisson, etc.       | required        |
| `rayon`       | Optional parallel Monte Carlo helper     | optional (`monte-carlo` feature) |
| `criterion`   | Statistical benchmark harness            | dev-dependency  |

Error types will be added via `thiserror` when a public fallible API is
introduced (first candidate: process-join in post-MVP). Until then the library
surface is panic-on-misuse only.

No async runtime dependency (tokio, async-std) — the custom executor is self-contained.

---

## 9. Out of Scope for MVP

- Real-time synchronisation.
- Networked / distributed simulation.
- GUI or visualisation.
- Monitoring / statistics collection (left to the application layer).
- GPU acceleration.
- Event recording and replay (seeded determinism already makes replay redundant;
  re-running with the same seed reproduces the run bit-for-bit).
- Process interrupts and preemption.
- `Container`, `Store`, `FilterStore` resource types.

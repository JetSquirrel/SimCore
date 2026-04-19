# Implementation Plan

Each step adds a self-contained increment of functionality and extends `examples/hospital.rs` to demonstrate it.

---

## Step 1: SimEnv + Timeout + Executor ✅

The foundational heartbeat of the library.

**Files:**
- `src/executor/queue.rs` — min-heap event queue with tie-breaking sequence numbers
- `src/executor/waker.rs` — custom `Waker` implementation
- `src/executor/mod.rs` — event loop that drives future polling
- `src/env.rs` — `SimEnv`: `spawn()`, `run()`, `run_until()`, `now()`, `timeout()`; `EnvHandle` for use inside processes
- `src/timeout.rs` — `Timeout` future

**Hospital example:** a handful of patients arrive at fixed intervals, each sleeping for a fixed treatment duration, printing their discharge time.

---

## Step 2: Manual Event ✅

Adds inter-process signalling.

**Files:**
- `src/event.rs` — `EventTrigger` / `EventAwaitable`

**Design details:**
- Multiple processes can await the same `EventAwaitable` (multi-waiter via `Vec<Waker>`).
- `EventTrigger::fire()` drains all registered wakers in one pass.
- `fired` latch: if `fire()` is called before any process awaits, the awaitable resolves immediately on the first poll.

**Hospital example:** a triage nurse process signals a patient process when assessment is complete.

---

## Step 3: Resource ✅

Adds shared, capacity-limited resources with RAII acquisition.

**Files:**
- `src/resource/mod.rs` — `Resource`, `ResourceRequest` (future), `ResourceGuard` (RAII)

**Design details:**
- `Resource` wraps `Rc<RefCell<ResourceState>>` and is `Clone`.
- Waiters are served FIFO via `VecDeque<Waker>`.
- `ResourceGuard::drop` pops and wakes exactly one waiter.
- `Resource::new(0)` panics with `"capacity must be at least 1"`.

**Hospital example:** patients contend for a limited pool of beds and doctors.

---

## Step 4: Seeded RNG ✅

Makes simulations stochastic and reproducible.

**Changes:**
- `SimEnv::with_seed(seed: u64)` constructor.
- `SimEnv::new()` seeds from OS entropy.
- `EnvHandle::rng()` returns a `RngGuard` newtype wrapping `RefMut<StdRng>`.
- `RngGuard` implements `RngCore` by delegation — safe to use with `rand_distr` distributions.
- Sampling must happen before `async move` blocks (guard cannot cross `.await`).

**Hospital example:** Poisson inter-arrival times and exponential service durations. Same seed → identical output.

---

## Step 5: Monte Carlo ✅

Adds parallel multi-run support.

**Files:**
- `src/monte_carlo.rs` — `run(seeds, f) -> Vec<R>` using `Arc<F>` + `std::thread::spawn`

**Design details:**
- `SimEnv` is `!Send`, so it is created *inside* each closure — never crosses thread boundaries.
- Results are returned in seed order (one element per seed).

**Hospital example:** runs 10 simulations with different seeds; each run writes to `hospital_run_<N>.log`; `main()` prints a summary table of patient throughput and resource wait times.

---

## Step 6: Test Suite ✅

Comprehensive integration and unit tests, plus code coverage analysis.

**Files:**
- `tests/timeout.rs` — 8 tests covering the executor and `Timeout` future
- `tests/event.rs` — 5 tests covering `EventTrigger` / `EventAwaitable`
- `tests/resource.rs` — 6 tests covering `Resource`, `ResourceGuard`, FIFO ordering
- `tests/system.rs` — 3 system tests (Job Shop with Quality Gate, determinism, Monte Carlo)
- `src/executor/queue.rs` — 3 inline unit tests for `ScheduledWaker::PartialEq` (type is `pub(crate)`)
- `TESTING.md` — test strategy, coverage commands, coverage table, benchmark guide

**Coverage:** 98.6% lines / 97.7% regions across all library source files.

---

## Step 7: Performance Benchmarks ✅

Criterion-based benchmark suite for regression detection and throughput measurement.

**Files:**
- `benches/simulation.rs` — five parameterized benchmark groups
- `Cargo.toml` — added `criterion = "0.5"` dev-dependency and `[[bench]]` entry

**Benchmark groups:**

| Group | Scenario | N values |
|---|---|---|
| `timeout_throughput` | N processes each `timeout(i).await` — raw executor baseline | 1K / 10K / 100K |
| `resource_contention` | N processes queue for 1-capacity resource, hold 1 tick each | 100 / 1K / 10K |
| `event_broadcast` | N processes await one event; fired simultaneously | 100 / 1K / 10K |
| `mixed_workload` | N patients: nurse (cap 1) → hold 5t → beds (cap 3) → hold 20t | 100 / 1K / 10K |
| `monte_carlo_scaling` | K threads, each running `mixed_workload(100)` | 1 / 2 / 4 / 8 |

HTML reports: `target/criterion/`. Baseline workflow: `--save-baseline main` then `--baseline main`.

---

## Step 8: PriorityResource ✅

Priority-ordered resource pool: lowest priority number served first; FIFO tie-breaking within the same priority level.

**Files:**
- `src/resource/priority.rs` — `PriorityResource`, `PriorityResourceRequest`, `PriorityResourceGuard`
- `src/resource/mod.rs` — added `pub use priority::...`
- `src/lib.rs` — re-exported `PriorityResource`, `PriorityResourceGuard`, `PriorityResourceRequest`
- `tests/priority_resource.rs` — 6 tests
- `examples/hospital.rs` — nurse changed to `PriorityResource`; patients get triage levels (critical=0, standard=1)

**Design:**
- `BinaryHeap<PriorityWaiter>` with reversed `Ord` so `pop()` yields the lowest `(priority, seq)` pair
- `next_seq` counter in state for FIFO tie-breaking within the same priority level
- `registered` + `seq` on the future — same double-registration guard as `ResourceRequest`
- API: `r.request(priority: u32) -> PriorityResourceRequest` — priority always explicit at call site

---

## Step 9: AnyOf / AllOf Combinators ✅

Race and barrier combinators: `AnyOf` resolves when the first sub-future fires;
`AllOf` resolves when all sub-futures have fired.

**Files:**
- `src/combinator.rs` — `AnyOf`, `AllOf`, `any_of!` and `all_of!` macros
- `src/timeout.rs` — bug fix: `poll` now checks `scheduled && now() >= deadline`
  instead of just `scheduled`; needed so `AllOf` can re-poll timeouts without
  false positives
- `src/lib.rs` — added `pub mod combinator`; re-exported `AnyOf`, `AllOf`
- `tests/combinator.rs` — 8 tests
- `examples/hospital.rs` — added bed-pressure monitor + `any_of!` in patient
  treatment; added `early_discharged` stat and summary column

**Design:**
- `AnyOf::poll`: iterates sub-futures, returns `Ready` on first that resolves
- `AllOf::poll`: uses `Vec::retain_mut` to drop completed futures; returns `Ready`
  when list is empty
- Both types are `Unpin` (only contain `Vec` and `Pin<Box<...>>`), so `get_mut()`
  is safe in `poll`
- Macros auto-`Box::pin` each expression, eliminating call-site verbosity

---

---

## Step 10: Container ✅

Continuous-quantity resource: a reservoir with `level: f64` and `capacity: f64`.
Both `put(amount)` and `get(amount)` are suspendable futures (FIFO queues).
Level changes are committed eagerly by the wake cascade via an `Rc<Cell<bool>>`
done-flag shared between the future and its waiter entry.

**Files:**
- `src/resource/container.rs` — `Container`, `ContainerPutRequest`, `ContainerGetRequest`,
  `ContainerState`, `GetWaiter`/`PutWaiter`, `wake_get_waiters`, `wake_put_waiters`, `trigger_cascade`
- `src/resource/mod.rs` — added `pub mod container` + re-exports
- `src/lib.rs` — re-exported `Container`, `ContainerGetRequest`, `ContainerPutRequest`
- `tests/container.rs` — 11 integration tests
- `examples/hospital.rs` — blood bank sub-scenario (Container with restocking process)

**Design:**
- `put` blocks when `level + amount > capacity`; `get` blocks when `level < amount`
- FIFO `VecDeque` for both get and put waiter queues
- `Rc<Cell<bool>>` done-flag: cascade commits level change and sets flag before waking,
  so re-polled future returns `Ready` without rechecking level (preserves FIFO)
- `trigger_cascade` loops `wake_get_waiters`/`wake_put_waiters` until stable (handles
  cascading chains where a put immediately enables a get enables another put, etc.)

---

## Step 11: ProcessHandle ✅

`spawn` now returns `ProcessHandle<F::Output>` — a tokio-`JoinHandle`-style
future resolving to the process's return value. Enables return values from
processes, process-level `any_of!`/`all_of!` composition, and observable
completion.

**Files:**
- `src/process.rs` — `ProcessHandle<T>`, `ProcessSlot<T>`, `spawn_with_handle()`
  factory, `discard()` adapter for combinators
- `src/env.rs` — generalized `spawn` signatures (both `SimEnv` and `EnvHandle`)
- `src/lib.rs` — `pub mod process;` and `pub use ProcessHandle`
- `tests/process_handle.rs` — 9 integration tests
- `examples/hospital.rs` — `arrivals` collects patient handles; on shift end,
  `AllOf` joins them (with a hard `any_of` deadline) to log "ED clear" time

**Design:**
- Non-`Clone`, single-await — broadcast patterns use `EventTrigger`.
- Completion via `async move` wrapper that stores result in
  `Rc<RefCell<ProcessSlot<T>>>` and wakes a registered awaiter. No executor
  changes — the process table stays `HashMap<_, Pin<Box<dyn Future<Output=()>>>>`.
- Waker dedup via `Waker::will_wake` (same pattern as `EventAwaitable`).
- Panic semantics unchanged: process panic crashes the sim.
- Breaking change to `spawn` signature was source-compatible for every
  existing callsite in tests, benches, and examples (the returned
  `ProcessHandle<()>` is dropped at the statement boundary).

---

## Post-MVP Roadmap

Listed in priority order (see [SPEC.md §6](SPEC.md) for full details):

1. **`PreemptiveResource`** — higher-priority request can preempt a current holder.
2. **`Interrupt`** — one process can interrupt another (e.g., emergency preemption).
3. **`RealtimeEnvironment`** — synchronise simulated time to wall-clock time.
4. **`Store` / `FilterStore`** — discrete-item queues with optional filter predicate.
5. **GPU/CUDA acceleration** — batch evaluation of independent sub-simulations on GPU.

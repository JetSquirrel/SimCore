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
- `src/error.rs` — `SimError`

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

## Post-MVP Roadmap

Listed in priority order (see [SPEC.md §6](SPEC.md) for full details):

1. **`PreemptiveResource`** — higher-priority request can preempt a current holder.
2. **`AnyOf` / `AllOf` combinators** — wait for the first/all of a set of events.
3. **`ProcessHandle`** — await the completion of a spawned process.
4. **`Interrupt`** — one process can interrupt another (e.g., emergency preemption).
5. **Event recording and replay** — log all events with timestamps for deterministic debugging.
6. **`RealtimeEnvironment`** — synchronise simulated time to wall-clock time.
7. **`Container`** — continuous-quantity resource (e.g., blood supply in litres).
8. **`Store` / `FilterStore`** — discrete-item queues with optional filter predicate.
9. **GPU/CUDA acceleration** — batch evaluation of independent sub-simulations on GPU.

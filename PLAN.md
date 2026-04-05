# Implementation Plan

Each step adds a self-contained increment of functionality and extends `examples/hospital.rs` to demonstrate it.

---

## Step 1: SimEnv + Timeout + executor

The foundational heartbeat of the library.

Files:
- `src/executor/queue.rs` — min-heap event queue with tie-breaking sequence numbers
- `src/executor/waker.rs` — custom `Waker` implementation
- `src/executor/mod.rs` — event loop that drives future polling
- `src/env.rs` — `SimEnv`: `spawn()`, `run()`, `run_until()`, `now()`, `timeout()`
- `src/timeout.rs` — `Timeout` future
- `src/error.rs` — `SimError`

Hospital example: a handful of patients arrive at fixed intervals, each sleeping for a fixed treatment duration, printing their discharge time.

---

## Step 2: Manual Event

Adds inter-process signalling.

Files:
- `src/event.rs` — `EventTrigger` / `EventAwaitable`

Hospital example: a triage nurse process signals a patient process when assessment is complete, demonstrating one process waking another.

---

## Step 3: Resource

Adds shared, capacity-limited resources with RAII acquisition.

Files:
- `src/resource/mod.rs` — `Resource`, `ResourceRequest`, `ResourceGuard`

Hospital example: patients contend for a limited pool of beds and doctors. Shows queuing behaviour when all units are occupied.

---

## Step 4: Seeded RNG

Makes simulations stochastic and reproducible.

Changes:
- `with_seed()` constructor on `SimEnv`
- RNG handle accessible to processes

Hospital example: Poisson inter-arrival times and exponential service durations sampled per patient. Same seed produces identical output.

---

## Step 5: Monte Carlo

Adds parallel multi-run support.

Changes:
- Runner that spawns independent `SimEnv` instances on OS threads

Hospital example: runs 10 simulations with different seeds, collects per-resource wait times and patient throughput, prints a summary table.

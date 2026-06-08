# Architecture Review — 2026-06-08

**Reviewer:** Opus 4.8
**Scope:** Full architecture & code review of `simu` ahead of growing the library / building larger
dependent projects.
**Baseline:** 69 passing tests, clippy-clean (`-D warnings`), MVP complete.

**Stated priorities (from the requester):**

1. Convenient API offering SimPy-like functionality, but idiomatic Rust.
2. Designed for high performance, balanced against maintainability and extensibility.

**Overall verdict:** Strong, well-architected library. The core design — single-threaded async
executor over *simulated* time, `Rc<RefCell>` sharing, deterministic `(time, seq)` event ordering —
is the right approach and is cleanly executed. Test coverage is thorough (drop-safety, FIFO edge
cases, cascade chains). Almost everything below is polish, **except one confirmed correctness bug**
(Finding 1) discovered by targeted probing.

---

## Status legend

| Status | Meaning |
|--------|---------|
| OPEN | Not yet addressed |
| FIXED | Resolved in this review cycle |
| ACK | Acknowledged, deferred by decision |

---

## 1. Container can strand a serviceable put-waiter — **FIXED**

**Severity: High** (silent incorrect results — no panic). File: `src/resource/container.rs`.

The two immediate-completion paths were asymmetric:

- `ContainerGetRequest::poll` immediate path called `trigger_cascade` (both get **and** put waiters).
- `ContainerPutRequest::poll` immediate path called only `wake_get_waiters`.

So when an immediate `put` raised the level and woke a blocked `get`, that get could drain the level
back down — freeing space a **blocked put-waiter** could now use — but `wake_put_waiters` was never
called. The put-waiter was stranded despite being serviceable.

**Reproduction** (level 6 / cap 10; blocked `put(5)`; blocked `get(8)`; trigger `put(2)`):

```
log = ["trigger put @1", "G_block served @1"]   // P_block missing
>>> P_block was STRANDED (serviceable but never woken)
```

Existing tests missed it because no test had a blocked put-waiter and a blocked get-waiter coexisting
when an *immediate* put arrived.

**Fix:** the immediate `put` path now calls `trigger_cascade` (symmetric with `get`). Regression test
`immediate_put_wakes_blocked_put_after_get_drains` added.

## 2. `trigger_cascade` fragile float-epsilon termination — **FIXED**

**Severity: Medium** (latent; not independently reproduced, but unsound by construction). File:
`src/resource/container.rs`.

The cascade loop terminated when the net level delta across one pass was `< f64::EPSILON`. If a pass
drained `g` via gets and added exactly `g` via puts (net ≈ 0) while leaving a newly-serviceable
waiter, it could stop early. It also relies on `f64::EPSILON` as a "did anything change" proxy, which
is the classic float-tolerance trap.

**Fix:** `wake_get_waiters` / `wake_put_waiters` now report whether they serviced anyone (`bool`), and
`trigger_cascade` loops while *any* waiter was serviced in the pass — no float comparison involved.
Regression test `cascade_terminates_on_net_zero_level_delta` added.

---

## 3. API ergonomics (vs SimPy, Rust-idiomatic) — **PARTIALLY OPEN**

The API is already clean. Targeted suggestions:

- **Missing `#[must_use]`** on request/future constructors and accessors (`Resource::request`,
  `Container::get`/`put`, `AnyOf`/`AllOf`, `now()`/`level()` …). Pedantic clippy flagged ~26 sites.
  **FIXED:** all 26 public `src/` sites now carry `#[must_use]` — future-returning methods use an
  "futures do nothing unless awaited" hint, accessors/constructors a plain attribute. `spawn` is
  deliberately left un-annotated (dropping its `ProcessHandle` is the idiomatic detach). Six
  `#[should_panic]` constructor tests were updated to bind the now-`must_use` result.
- **Doc-tests are all `ignore`d** (`combinator.rs`, `env.rs`, …). They are not compiled, so examples
  can silently rot. Converting a few to real compiling doc-tests would protect the public surface.
- **`PriorityResource::request(priority: u32)`** uses a magic integer. A `Priority(u32)` newtype or
  `impl Into<u32>` would read better. Minor; current form mirrors SimPy.
- **`Container::get`/`put` return `()`** rather than the amount (SimPy returns it). Fine for now, but
  worth revisiting if partial-fill semantics are added.

## 4. Performance observations — **OPEN (optimizations, not defects)**

Hot path is solid (`BinaryHeap` event queue + `HashMap` process table + waker re-queue). Gate any
change behind the existing Criterion suite.

- **`poll_ready` removes/re-inserts each pending process from the `HashMap` every poll**
  (`env.rs`). For thousands of processes polled many times this is repeated hashing + realloc churn.
  Process ids are dense `usize` from a counter, so a `slab`/`Vec`-indexed table would be O(1) without
  hashing and improve cache locality.
- **Linear waker scans** in `EventAwaitable` (`will_wake`) and `AnyOf`/`AllOf`. Fine for small fan-in;
  `event_broadcast` at n=10k is the case to watch.

## 5. Robustness / smaller issues — **PARTIALLY OPEN**

- **Process panics abort the entire run** (verified: a panic in one process unwinds out of
  `env.run()`, stranding all others). SPEC §4.3 said *"Panicking inside a process terminates that
  process and propagates as a simulation error"*, which did not match the whole-run abort.
  **FIXED (docs):** SPEC §4.3 now states the true whole-run-abort behaviour, and per-process
  `catch_unwind` isolation is tracked as roadmap item §6.6. The behavioural change itself remains
  deferred. — DOCS RECONCILED; behaviour OPEN.
- **`run_until` sets `current_time = until`** even when the queue empties earlier (`env.rs`). Defensible
  (time advances to the boundary) but subtle: a process scheduled exactly at `until` is not run, yet
  `now()` reports `until`. Worth a doc note. — OPEN.

## 6. Maintainability / extensibility — **OPEN**

- **Four request-futures (`Resource`, `PriorityResource`, `Container` get/put) duplicate the
  `registered` / `canceled` / drop-cancel pattern nearly verbatim.** Adding `PreemptiveResource` /
  `Store` will copy it again. Extracting a small internal `WaitQueue` helper (queue + cancel-skip +
  FIFO seq + cascade) that each resource composes is the biggest *future* maintainability win — and
  would have made Findings 1 & 2 structurally impossible (one cascade implementation, not two).
  **Strongly recommended before adding post-MVP resource types.**
- **Module privacy:** tests/benches import `simu::env::SimEnv` (module path) rather than the curated
  re-export `simu::SimEnv`. Consider `pub(crate)` on the modules and re-export only the public surface
  from `lib.rs`.

---

## Recommended action order

1. **Fix the Container put-cascade bug + regression test** (Finding 1) — *done this cycle.*
2. **Harden `trigger_cascade` termination** (Finding 2) — *done this cycle.*
3. **Reconcile panic-semantics docs** in SPEC §4.3 (Finding 5) — *done; truthfulness restored, behaviour deferred to §6.6.*
4. **Add `#[must_use]`** to futures/accessors (Finding 3) — *done; 26 sites annotated.*
5. *(When growing)* extract the shared `WaitQueue` helper (Finding 6) before adding
   `PreemptiveResource` / `Store`.

## Changelog for this cycle

- Finding 1 — FIXED: immediate `put` path now runs the full `trigger_cascade`.
- Finding 2 — FIXED: cascade termination now driven by "work done" flags, not float-epsilon.
- Tests — 2 regression tests added to `tests/container.rs`
  (`immediate_put_wakes_blocked_put_after_get_drains`,
  `cascade_terminates_on_net_zero_level_delta`). Both were confirmed to **fail** against the
  pre-fix code and **pass** after the fix. Suite is now 71 passing, clippy-clean with and without
  the `monte-carlo` feature.

# Implementation Review — 2026-07-01

**Reviewer:** Fable 5 (limited-availability deep-review pass)
**Scope:** Full review of implementation, documentation, test coverage, and examples, ahead of the
planned open-source release (see `PUBLISHING.md`).
**Baseline:** 103 passing tests + 2 doc-tests, clippy-clean (`--all-targets -D warnings`), MVP +
`PreemptiveResource` + external random feed complete.
**Prior review:** `reviews/2026-06-08-architecture-review.md` (all its FIXED items verified still in place).

**Execution note (per requester):** implementation of these improvements will be done by
**Opus 4.8** or **Sonnet**. Every finding carries a **`Model:`** line. "Sonnet" means the change is
mechanical or well-specified enough for Sonnet with this document as the spec; "Opus" means it
touches load-bearing executor/queue semantics, needs invariant reasoning across files, or has
subtle drop-order/cancellation interactions.

**Overall verdict:** The architecture remains excellent and the code is unusually well-commented.
However, this pass found **one confirmed high-severity deadlock** (Finding 1 — empirically
reproduced, trace below) in the shared wake-and-retry protocol that all three unit-based resources
sit on, plus two confirmed low/medium clock-monotonicity bugs. The rest is API hardening,
doc rot, and perf polish — all of which are cheapest to do **now, before the crates.io release
freezes the public API**.

---

## Status legend

| Status | Meaning |
|--------|---------|
| OPEN | Not yet addressed |
| FIXED | Resolved in a later cycle (update this file when done) |
| ACK | Acknowledged, deferred by decision |

---

## Resolution log — 2026-07-01 (Opus 4.8, batch after F1)

All findings below were addressed in one pass on branch `fix/review-2026-07-01`,
following the recommended execution order. Gate after the batch: **132 tests + 7
doc-tests green** on both the default and `--features monte-carlo` builds, clippy
clean (`--all-targets -D warnings`) on both, all three examples run, benches
build, and the exact-mode `compare` models stay deterministic.

| Finding | Status | Note |
|---------|--------|------|
| F1 | **FIXED (verified)** | Direct handoff; verified by Fable 5. |
| F2 | **FIXED** | `run_until` clamps `until.max(now)`; tests + doc. |
| F3 | **FIXED** | `timeout` panics on negative/NaN/∞; `debug_assert` in `schedule_wakeup`; tests + doc. |
| F4 | **FIXED** | `ScheduledWaker::Ord` now uses `f64::total_cmp`. |
| F5 | **FIXED** | `Container::put`/`get` panic on `amount > capacity`; tests + doc. |
| F6 | **FIXED (docs)** | Unfired-`EventTrigger`-drop semantics documented; regression test added. |
| A1 | **FIXED** | Crate-level `//!` docs + compiling quick-start; `#![warn(missing_docs)]` (no gaps). |
| A2 | **FIXED** | Modules `env`/`event`/`timeout`/`process`/`combinator`/`resource` are now private; only the curated root surface + `rng`/`monte_carlo` are public; macros use `$crate::AnyOf`/`AllOf`. |
| A3 | **FIXED** | All four `ignore` doc-tests converted to compiling doc-tests (0 ignored). |
| A4 | **DONE (rand 0.9)** | 0.10 deferred (larger reorg); see the A4 section. |
| A5 | **FIXED** | `monte_carlo::run` uses `thread::scope`; dropped `'static` + `Arc`. |
| A6 | **FIXED** | `queue_len()` on the three pools; `get_queue_len`/`put_queue_len` on `Container`. |
| A7 | **FIXED** | `Debug` on all public types; `set_seed(&mut self)`; `Timeout` deadline-at-creation documented; `Priority` newtype left ACK. |
| P1 | **FIXED** | One cached `Waker` per process (created at admission); `will_wake` dedup now effective. |
| P2 | **FIXED** | Process table is `Vec<Option<ProcessEntry>>` indexed by dense id; O(1) take/put-back, no hashing. |
| P3 | **FIXED (comment)** | Stale-id poll documented as benign in `poll_ready`. |
| D1 | **FIXED** | SPEC §2 (naming) and §9 (out-of-scope) reconciled with shipped state. |
| D2 | **FIXED** | TESTING.md `VecDeque` rot removed; coverage table replaced with a regen command; counts/benches refreshed. |
| D3 | **FIXED** | README import path, license, and install snippet updated. |
| D4 | **FIXED** | PUBLISHING.md MSRV corrected to 1.82 (`Option::is_none_or`). |
| D5 | **FIXED** | Behaviour notes landed with F2/F3/F5/F6 and in API.md. |
| T1 | **FIXED** | `tests/same_tick_races.rs` (from F1). |
| T2 | **FIXED** | `tests/adversarial_scheduling.rs` — same-batch double-release, preempt-during-batch, mixed container. |
| T3 | **FIXED** | `priority_contention`/`preemptive_contention`/`container_throughput` bench groups. |
| T4 | **FIXED** | `monte_carlo_propagates_worker_panic` (both backends). |
| E1 | **FIXED** | `hospital` distinguishes the safety-net-deadline arm from a real ED clear. |
| E2 | **PARTIAL** | Eviction comment + `compare.rs` in CLAUDE.md done; shared `log_dir` extraction left (cosmetic). |

---

# Part 1 — Correctness (implementation)

## F1. Woken waiter stranded forever when it loses the same-batch acquire race — **FIXED (verified)**

> **2026-07-01 (Opus 4.8):** Direct-handoff fix implemented per the recommendation
> below. `WaitQueue::release` now transfers the unit (sets a shared `granted`
> flag, leaves `in_use` unchanged) instead of freeing it; requests check `granted`
> first and carry a `consumed` flag so a granted-then-dropped request releases
> exactly once. Applied to `Resource`, `PriorityResource`, `PreemptiveResource`.
> New `tests/same_tick_races.rs` reproduces the deadlock/FIFO-violation and is
> confirmed to **fail on the pre-fix `release()`** and pass after. Full suite
> green, clippy clean on both feature sets. Exact-mode compare models
> (`mm1`/`mmc`/`priority`/`container`) and `hospital` produce byte-identical
> output before/after the fix, so SimPy parity is preserved (SimPy itself was not
> re-run — not installed in this environment; verified via before/after diff of the
> `compare` example instead). SPEC §4.7 and TESTING.md updated.
>
> **2026-07-01 (Fable 5, independent verification): fix confirmed correct.**
> Re-read all four changed files against the protocol; re-derived the load-bearing
> invariant (while any live waiter is queued, `in_use` never drops below capacity,
> so an unconditional `try_acquire` on a spurious re-poll can never succeed — this
> also closes the `PreemptiveRequest` re-poll corner, since a strictly-worse-priority
> holder is unreachable while a waiter is parked: handoff grantees and preemptors
> both acquire at priority ≤ the parked waiter's). Independently re-ran the
> discrimination check (temporary revert of `release()`): the three same-batch
> tests **fail pre-fix** as claimed. Found one coverage gap: the original
> granted-then-dropped test listed the request arm *first* in its `any_of!`, so
> the grant was consumed into a guard before the timeout arm resolved — the
> `granted && !consumed` Drop branch (the subtlest part of the fix) was never
> executed (confirmed by instrumentation). Added
> `resource_granted_but_unconsumed_drop_passes_unit_on` (timeout arm first),
> confirmed via instrumentation that it exercises that branch, and confirmed it
> **also fails pre-fix** (it catches a second pre-fix bug flavor: a wake consumed
> by an abandoning waiter stranded the live waiter parked behind it). Final gate:
> 111 tests green on both feature sets, clippy clean, compare models byte-identical
> to both stored pre-fix and post-fix artifacts. No further changes required.

**Severity: HIGH — reachable deadlock + FIFO-contract violation.**
Files: `src/resource/wait_queue.rs` (root cause), `src/resource/mod.rs`,
`src/resource/priority.rs`, `src/resource/preemptive.rs` (all three resources affected).
**Model: Opus** (load-bearing protocol change with drop/cancel interactions).

### The bug

`WaitQueue::release()` **pops** the next live waiter's entry off the heap and wakes it
(`wait_queue.rs:131-139`). The woken request is expected to re-poll and grab the freed unit via the
unconditional `try_acquire()` at the top of `poll`. The comment at `wait_queue.rs:99-103` claims
this cannot jump the queue "because `release` wakes only the single next-in-line waiter" — but the
freed capacity is visible to **everyone** polled between the release and the woken waiter's
re-poll. If any other process in the *same ready batch* issues a fresh `request()`, its first poll
`try_acquire`s the unit out from under the woken waiter. The woken waiter then re-polls, fails,
and — because it is `registered == true` but its heap entry is **already popped** — it never
re-registers and its waker is spent. It is stranded **forever**, even after the thief releases
(the release finds an empty heap and wakes nobody).

### Empirical reproduction (verified 2026-07-01 against this tree)

Capacity-1 `Resource`. P0 holds the unit and waits on an `EventAwaitable`; A is the blocked FIFO
waiter; Q waits on the *same* event and requests after P0 releases. `trigger.fire()` puts P0 and Q
into one ready batch; P0's guard-drop wakes A into the *next* batch; Q's fresh request steals:

```
trace: ["P0 released @1", "Q acquired @1", "Q released @2"]
        → A NEVER acquires. env.run() terminates with A suspended. DEADLOCK.
```

Two distinct defects in one:

1. **FIFO violation** (always): Q jumps ahead of A even though A queued first — contradicting the
   documented contract of `Resource::request` ("woken in FIFO order"), `PriorityResource`, and
   SPEC §4.7.
2. **Permanent stranding** (when the thief holds the guard across any yield): A's wake is consumed,
   its entry is gone, `registered` blocks re-registration → deadlock that survives to the end of
   the run. `env.run()` silently returns with the process suspended, so the user just sees wrong
   metrics (a job that never completes), not a crash.

Note the existing probe `woken_waiter_losing_same_tick_race_is_not_starved`
(`tests/preemptive_resource.rs`) does **not** cover this: there the releaser and the fresh
requester are woken by two *separate* timeout events, and `poll_ready` fully drains between events,
so the woken waiter is always polled first. The race requires both to be in **one ready batch** —
easiest via one `EventTrigger::fire()` waking both, which is an entirely ordinary usage pattern.

### Recommended fix: direct handoff (commit-at-wake), matching SimPy and matching `Container`

`Container` is immune because its cascade *commits* the level change and sets a `done` flag before
waking (`container.rs`, `done: Rc<Cell<bool>>`). Bring `WaitQueue` to the same model:

- Add a `granted: Rc<Cell<bool>>` to each waiter `Entry` (shared with the request future, exactly
  like `canceled`).
- `release()`: instead of `in_use -= 1` + pop + wake, **transfer** the unit: pop the next live
  entry, set its `granted` flag, wake it, and leave `in_use` unchanged (the unit never becomes
  "free", so no fresh request can steal it). Only when no live waiter exists does `in_use -= 1`.
- Request `poll()`: check `granted` **first** — if set, return `Ready(guard)` without touching
  `try_acquire`. The initial-poll fast path (`try_acquire` when not yet registered) stays.
- **The subtle part** (why this is Opus work): a request that is granted-but-not-yet-repolled can
  be **dropped** (losing `any_of!` arm). Its `Drop` currently just sets `canceled`; with handoff it
  must detect `granted && !consumed` and call `release()` itself so the unit is passed on — the
  same shape as `ContainerGetRequest::drop`'s `registered && !done` check, but with a real
  release-side effect. Also mirror the logic in `PreemptiveRequest` (its blocked path goes through
  the same `WaitQueue`) and verify `PreemptiveGuard::drop`'s `wq.release()` still balances
  `holders` bookkeeping when release becomes a transfer.
- Keep FIFO on the fresh-request fast path: with handoff, `in_use` never dips while waiters exist,
  so a fresh request naturally fails `try_acquire` and queues behind them — no extra
  "has live waiter" gate is needed (unlike `Container`). Convince yourself of this and document it.

### Required regression tests (add to the fix)

- The reproduction above, for `Resource`, `PriorityResource`, and `PreemptiveResource`
  (same-batch steal via a shared event; thief holds across a yield; assert the FIFO waiter still
  completes AND completes *before* the fresh requester).
- Granted-then-dropped: waiter wins the handoff but its future is dropped before re-poll (via
  `any_of!` where a timeout arm wins at the same tick) → assert the unit is passed to the next
  waiter, `in_use` stays consistent, and no double-release.
- Re-run the SimPy parity harness (`compare/run_comparison.sh --no-perf`): handoff semantics are
  *closer* to SimPy, so exact-mode models must stay green; check whether
  `hospital.early_discharged` (the `KNOWN_EXCEPTIONS` entry) changes and update the allowlist and
  the memory/docs if the divergence disappears.

## F2. `run_until(t)` with `t` in the past rewinds the clock — **OPEN**

**Severity: LOW-MEDIUM (silent wrong results).** File: `src/env.rs:157-183`.
**Model: Sonnet.**

Verified: after `env.run()` reaches `now() == 10.0`, calling `env.run_until(5.0)` sets
`now() == 5.0`. Simulated time must be monotonic. Fix: in the `should_stop` branch, set
`current_time = until.max(state.current_time)` (one line), and document in `run_until`'s rustdoc
that (a) the clock advances *to* `until` when the queue empties early, and (b) a boundary in the
past is a no-op. Add two small tests (`run_until` past boundary; `run_until` early-empty queue
jumps to the boundary — the second behaviour exists but is untested).

## F3. Negative (and NaN) timeout delays rewind / wedge the clock — **OPEN**

**Severity: MEDIUM (violates the crate's own "programming error ⇒ panic" policy).**
Files: `src/env.rs:286-289` (`EnvHandle::timeout`), `src/executor/mod.rs:46-50` (`schedule_wakeup`).
**Model: Sonnet.**

Verified: `h.timeout(-5.0)` at `t=10` schedules a wakeup at `t=5`; the event loop pops it and sets
`current_time = 5.0` — **global time goes backwards for every process** (trace:
`["before: 10", "after: 5"]`). A NaN delay is worse: the `ScheduledWaker` ordering treats NaN as
`Equal` to everything (F4), and `Timeout::poll`'s `now() >= deadline` is never true — the process
silently never resumes.

Fix per the error strategy (SPEC §3: wrong API use panics):

- `EnvHandle::timeout`: `assert!(delay >= 0.0 && delay.is_finite(), "timeout delay must be finite and non-negative (got {delay})")`.
  (Decide: allow `0.0` — yes, it's tested and documented.)
- Belt-and-braces in `SimState::schedule_wakeup`: `debug_assert!(time.is_finite() && time >= self.current_time)`.
- Tests: `#[should_panic]` for negative and NaN delays; keep `zero_delay_timeout` green.
- Document the panic in `timeout()` rustdoc (`# Panics` section) and API.md.

## F4. `ScheduledWaker::Ord` silently tolerates NaN, breaking heap invariants — **OPEN**

**Severity: LOW (latent; unreachable once F3 lands, but unsound by construction).**
File: `src/executor/queue.rs:28-35`. **Model: Sonnet (bundle with F3).**

`partial_cmp(...).unwrap_or(Ordering::Equal)` makes NaN compare `Equal` to *everything*, which
violates `Ord`'s transitivity/totality contract and can corrupt `BinaryHeap` ordering arbitrarily
(not memory-unsafe, but silently wrong pop order). After F3 guarantees finite times, replace
`unwrap_or(Equal)` with `f64::total_cmp` — it is total, correct for all finite values, and removes
the hidden NaN policy entirely. One-line change + keep existing unit tests.

## F5. `Container` accepts un-satisfiable requests (`amount > capacity`) that block the queue forever — **OPEN**

**Severity: LOW-MEDIUM.** File: `src/resource/container.rs:206-234`. **Model: Sonnet.**

`c.put(11.0)` or `c.get(11.0)` on a capacity-10 container can never complete. Under strict
head-of-line FIFO it parks at the queue head and **permanently blocks every later waiter** — a
whole-queue deadlock from one bad argument. This is a programming error by the crate's own policy:
add `assert!(amount <= capacity)` in `put`/`get` (message naming the amount and capacity),
document under `# Panics`, add two `#[should_panic]` tests, and mention in API.md. (SimPy also
blocks forever here; diverging is deliberate and should be noted in the SimPy-parity docs.)

## F6. Dropping an unfired `EventTrigger` silently strands waiters — document — **OPEN**

**Severity: LOW (doc gap, defensible behaviour).** File: `src/event.rs`. **Model: Sonnet.**

If an `EventTrigger` is dropped without `fire()`, all current and future awaiters of the paired
`EventAwaitable` wait forever (until `SimEnv::drop` reclaims them). That is a reasonable DES
semantic, but it is nowhere stated. Add one paragraph to `EventTrigger`'s rustdoc and a line in
API.md's Events section. *Optional* (discuss first, do not just implement): a
`debug_assert!`-style log or a `fire_on_drop` opt-in are alternatives; SimPy does nothing, so
documenting is enough.

---

# Part 2 — Executor performance / robustness

## P1. A fresh `Waker` is allocated per process per poll, defeating every `will_wake` dedup — **OPEN**

**Severity: MEDIUM (perf + memory growth on hot paths).** Files: `src/env.rs:225`
(`make_waker` inside the poll loop), `src/event.rs:70`, `src/process.rs:44`.
**Model: Opus** (executor core; interacts with F1's rework).

`poll_ready` calls `make_waker(id, …)` — a new `Arc<SimWaker>` — **every time** a process is
polled. Consequences:

- One `Arc` allocation per poll of every process (hot-path churn).
- `EventAwaitable::poll`'s `will_wake` dedup (`event.rs:70`) compares against a waker from a
  *previous* poll — always false — so a pending awaitable re-polled N times (e.g. as the losing arm
  inside `any_of!`/`all_of!` re-polls) accumulates **N duplicate wakers** in `waiters`. Each fires
  a redundant wake (handled gracefully, but O(N) wasted re-polls and unbounded `Vec` growth for
  long-lived events). The same defeat applies to `ProcessHandle`'s dedup (`process.rs:44`), which
  currently *replaces* the waker each poll — correct but needless clone.

Fix: store one `Waker` per process, created at spawn time, in the process table (e.g.
`processes: HashMap<usize, (Pin<Box<…>>, Waker)>` or alongside P2's slab entry) and pass
`&waker` to `Context::from_waker`. Then `will_wake` dedup works as designed and allocation drops to
one per process lifetime. Gate with the Criterion suite (`event_broadcast` and `mixed_workload`
are the sensitive groups; expect neutral-to-positive movement).

## P2. Process table churn: `HashMap` remove/re-insert on every poll — **OPEN**

**Severity: LOW-MEDIUM (perf only; carried over from the 2026-06-08 review §4).**
File: `src/env.rs:210-237`, `src/executor/mod.rs:25`. **Model: Sonnet** (with this spec; do it
*after* P1 so the storage is designed once).

Process ids are dense monotonic `usize`s, yet every poll does `HashMap::remove` + re-insert
(hashing + probe churn). Replace `processes: HashMap<usize, …>` with a
`Vec<Option<ProcessEntry>>` indexed by id (ids are never reused, so a plain growing `Vec` is
sufficient; use `slab` only if id reuse is ever introduced). `take()`/`put-back` becomes two O(1)
slot writes. Keep `pending_spawns` as is. Verify with `timeout_throughput` (100k) and
`mixed_workload` (10k) benches; document the memory trade-off (completed processes leave a `None`
slot for the run's duration — fine for simulation lifetimes, note it in a comment).

## P3. `poll_ready` may poll a stale id — verified benign, add a comment — **OPEN (trivial)**

**Severity: INFO.** File: `src/env.rs:221-235`. **Model: Sonnet.**

Duplicate wakes push the same id twice; the second `processes.remove(&id)` returns `None` and is
skipped — correct. Worth a one-line comment so the next refactor (P2) preserves the property, plus
it becomes an explicit `if let Some(slot)` invariant in the `Vec` design.

---

# Part 3 — API & packaging (do BEFORE the crates.io release)

These are breaking or surface-shaping changes. The release in `PUBLISHING.md` freezes the API at
`0.1.0`; this is the cheapest moment they will ever have.

## A1. No crate-level rustdoc — docs.rs landing page will be empty — **OPEN**

**Severity: HIGH for the release.** File: `src/lib.rs`. **Model: Sonnet.**

`lib.rs` is 21 lines of re-exports with **no `//!` documentation at all**. On docs.rs (which builds
automatically after publish) the front page — the single most-read page of the crate — will show
nothing but an item list. Write a crate-level doc comment containing: one-paragraph pitch (the
README intro is good source material), the quick-start example as a **compiling doc-test**, the
core-types table, the `!Send`/Monte-Carlo model in two sentences, and links to the three examples.
Also add `#![warn(missing_docs)]` (warn, not deny, to keep CI friction low) and fix whatever it
flags — most items are already documented.

## A2. Public module paths create a double API surface — **OPEN**

**Severity: MEDIUM (breaking change; carried over from 2026-06-08 §6).** Files: `src/lib.rs`,
`README.md`, `tests/*`, `benches/simulation.rs`. **Model: Sonnet.**

Every module is `pub`, so `simu::env::SimEnv` and `simu::SimEnv` are both public paths (README uses
the former, API.md the latter). Rustdoc shows everything twice and SemVer must hold both stable.
Fix: make `env`, `event`, `timeout`, `process`, `combinator`, `resource` private (`mod`) and
re-export the curated surface from the root; keep `rng` and `monte_carlo` as public modules (they
are namespaces: `rng::sample`, `monte_carlo::run`). Update README/tests/benches imports
(`simu::env::SimEnv` → `simu::SimEnv`). Compile-fail fallout is the review: anything that breaks
outside the crate was an accidental exposure.

## A3. Four doc-examples are ```ignore``` and can silently rot — **OPEN**

**Severity: LOW.** Files: `src/combinator.rs:106,124`, `src/env.rs:276`,
`src/resource/preemptive.rs:18`. **Model: Sonnet.**

`cargo test` reports "4 ignored" doc-tests. Convert the two combinator macros and the `rng()`
example into compiling doc-tests (wrap in a spawned process; hidden `#` setup lines keep them
short). The `preemptive.rs` module example is long — acceptable to leave as `ignore`, but at least
convert it to `no_run` + make it compile, so type drift is caught.

## A4. `rand 0.8` / `rand_distr 0.4` are one major version behind — decide before publishing — **DONE (rand 0.9; 0.10 deferred)**

> **2026-07-01 (Opus 4.8):** Upgraded to `rand 0.9` / `rand_distr 0.5` per the
> documented migration below: dropped `try_fill_bytes`/`rand::Error` from the
> `RngCore` impls (now blanket-provided via `TryRngCore`), `from_entropy` →
> `from_os_rng`, `gen::<f64>()` → `random::<f64>()` in the examples, and updated
> the `rng_guard_covers_all_rngcore_methods` test. SplitMix64 known-answer
> vectors are unchanged (pure integer code), so exact-mode SimPy parity is
> preserved. `rand 0.10`/`rand_distr 0.6` are also out but restructure the crate
> (e.g. `RngCore` is no longer re-exported from `rand`), a larger migration than
> analyzed here — **deferred**; revisit before/at 1.0. Suite green, clippy clean
> on both feature sets.

**Severity: MEDIUM (strategic).** Files: `Cargo.toml`, `src/rng.rs`, `src/env.rs`, examples.
**Model: Opus** (public-trait surface: `RandomSource: RngCore` leaks the rand version).

`rand 0.9`/`rand_distr 0.5` (early 2025) changed the very traits this crate re-exports through
`RandomSource`: `RngCore::try_fill_bytes` and `rand::Error` are gone, `from_entropy` →
`from_os_rng`, `gen::<T>()` → `random::<T>()` (`gen` is a reserved keyword in edition 2024, which
also blocks a future edition bump). Because `RandomSource: RngCore` is public, the rand version is
part of simu's public API — publishing on 0.8 means the first upgrade forces a breaking release.
Recommendation: upgrade to rand 0.9 **before** `0.1.0`. Scope: rewrite the `RngCore` impls for
`SplitMix64`/`RngGuard` (smaller trait now), swap deprecated constructors, migrate example `gen()`
calls, re-run the SplitMix64 known-answer tests (pure integer code — must not change) and the SimPy
parity harness. If deferred instead, record the decision in PUBLISHING.md as a known post-1.0 break.

## A5. `monte_carlo::run` demands `'static + Arc` where scoped threads need neither — **OPEN**

**Severity: LOW (ergonomics).** File: `src/monte_carlo.rs`. **Model: Sonnet.**

`F: … + 'static` forces callers to move/clone everything into the closure. `std::thread::scope`
(stable since 1.63) lets the default backend accept `F: Fn(u64) -> R + Send + Sync` **without**
`'static` and without the `Arc` wrap; rayon's `par_iter` never needed `'static` either. Relax the
bounds on both backends (drop `R: 'static` too), keep panic-propagation semantics (scope joins all
threads before unwinding — verify the "first panic payload" contract still holds and keep the
existing test). Purely widening, non-breaking.

## A6. No queue introspection on resources — **OPEN**

**Severity: LOW (API gap vs SimPy).** Files: `src/resource/*.rs`. **Model: Sonnet.**

SimPy exposes `len(resource.queue)`; simu offers only `in_use`/`capacity`. Every example hand-rolls
wait statistics. Add `queue_len(&self) -> usize` (count of non-canceled waiters) to `Resource`,
`PriorityResource`, `PreemptiveResource` (delegating to a new `WaitQueue::live_waiters()`), and
`Container::get_queue_len()`/`put_queue_len()`. Trivial, additive, and makes the examples cleaner.

## A7. Assorted API polish — **OPEN**

**Model: Sonnet** (all items). Each is small; batch them:

- **`Debug` impls** for all public types (`SimEnv`, `EnvHandle`, resources, guards, requests,
  handles) — expected hygiene for a published crate (`#[derive(Debug)]` where possible; manual
  `finish_non_exhaustive()` for the `Rc<RefCell<…>>` wrappers).
- **`Timeout` deadline-at-creation semantics**: `timeout(d)` computes `deadline = now + d` when
  *created*, not when first awaited. Document this in `timeout()`/`Timeout` rustdoc (one sentence);
  it matters for stored futures raced in `any_of!`.
- **`SimEnv::set_seed(&self)`** mutates through `&self` while `run` takes `&mut self` — take
  `&mut self` for consistency (breaking, so do it now or never).
- Consider a `Priority` newtype later; the bare `u32` mirrors SimPy and is ACCEPTABLE as-is
  (carried from 2026-06-08 §3 — recommend ACK, revisit post-1.0).

---

# Part 4 — Documentation

## D1. SPEC.md contradicts the implementation in two places — **OPEN**

**Model: Sonnet.** File: `SPEC.md`.

- **§2 Crate Naming**: "likely unclaimed on crates.io" — now known false; the publish name is
  `simu-des` with `[lib] name = "simu"` (see `PUBLISHING.md`). Rewrite the section to record the
  actual naming decision.
- **§9 Out of Scope for MVP** still lists "Process interrupts and preemption" and "`Container`,
  `Store`, `FilterStore` resource types" — but `Container` **and** `PreemptiveResource` are shipped
  and documented earlier in the same file. Reword §9 to reference the current state ("delivered
  post-MVP: …; still out of scope: `Store`/`FilterStore`, …").

## D2. TESTING.md rot — **OPEN**

**Model: Sonnet.** File: `TESTING.md`.

- Test-purpose lines still name the old implementation: `fifo_ordering_three_waiters` "(`VecDeque`
  push_back/pop_front)" (line ~131) — `Resource` has used the `WaitQueUE<()>` `BinaryHeap` since the
  wait-queue extraction. Same stale "VecDeque" claim in the `resource_contention` bench description
  (also in `benches/simulation.rs:37` itself).
- The coverage table (lines ~62-83) is explicitly stale ("last measured before the
  WaitQueue/Preemptive/rng additions"). Re-run `cargo llvm-cov --summary-only` and refresh, or
  replace the per-file table with just the command and the headline number.
- After F1-F5 land, the counts (86+17) and file lists need refreshing anyway — do it in the same PR.

## D3. README issues — **OPEN**

**Model: Sonnet.** File: `README.md`.

- Quick start imports `use simu::env::SimEmv`-style module path (`simu::env::SimEnv`) while API.md
  uses `use simu::SimEnv` — unify on the curated root path (required anyway by A2).
- "License: TBD" — resolved by `PUBLISHING.md` (MIT OR Apache-2.0); update when the LICENSE files
  land.
- The install snippet says `simu = { path = "." }` — after publishing this becomes
  `simu-des = "0.1"` with a note that the import path stays `use simu::…` (already specified in
  `PUBLISHING.md` §4; just don't forget the README is the file it edits).

## D4. MSRV: `PUBLISHING.md` suggests \~1.75, but the code requires ≥ 1.82 — **OPEN**

**Severity: MEDIUM (would fail `cargo publish` verification builds downstream).**
**Model: Sonnet.** Files: `PUBLISHING.md`, later `Cargo.toml`.

`src/env.rs:167` uses `Option::is_none_or`, stabilized in **Rust 1.82** (Oct 2024). The publishing
plan's placeholder "declare something safe like 1.75" is therefore wrong. Either declare
`rust-version = "1.82"` (fine; 1.82 is >18 months old at release time) or swap the one call site to
`map_or(true, …)` and audit for other recent-API uses (`retain_mut` in `combinator.rs` is 1.61 —
fine). Update `PUBLISHING.md` now so the wrong number doesn't get executed later; verify with
`cargo +1.82 build` (or `cargo msrv find`) before publishing.

## D5. Behaviour notes worth one sentence each — **OPEN**

**Model: Sonnet.** Bundle with the fixes that touch the same files:

- `run_until` clamp/no-rewind semantics (with F2).
- `timeout()` panic conditions (with F3) and deadline-at-creation (A7).
- `Container` amount ≤ capacity panic (with F5) + note the deliberate SimPy divergence.
- Unfired-trigger-drop semantics (F6).
- API.md "Key rules": "Cancelled requests: dropping … removes it from the queue" — technically it
  *marks the entry canceled; removal is lazy*. Reword to avoid implying eager removal (matters for
  `queue_len` in A6).

---

# Part 5 — Test coverage

Current coverage is genuinely strong (drop-safety, cancellation, cascade chains, cross-language
KATs). Gaps found:

## T1. Regression tests for every Part-1 finding — **OPEN**

**Model:** same model as the corresponding fix (the F1 suite is **Opus**, the rest **Sonnet**).
Enumerated inside F1-F5; the F1 same-batch race deserves a dedicated
`tests/same_tick_races.rs` covering all three unit resources plus the granted-then-dropped case.

## T2. Adversarial-scheduling test family — **OPEN**

**Severity: MEDIUM.** **Model: Opus** (designing the interleavings is the hard part; a good
follow-on for whoever fixes F1).

Every existing race test wakes contenders via separate timeout events, which the executor fully
drains between — so *same-batch* interleavings (the F1 class) are systematically unexplored. Add a
small helper that puts N processes into one ready batch via a shared `EventAwaitable`, then probe:
release-then-request, request-then-release, double-release-two-waiters, preempt-during-batch, and
`Container` put/get mixes in one batch. Consider a `proptest`-based randomized interleaving driver
as a stretch goal (dev-dependency only) — DES executors earn their keep under exactly these
schedules.

## T3. Bench coverage gaps — **OPEN (low)**

**Model: Sonnet.** File: `benches/simulation.rs`.

No benchmarks exercise `PriorityResource`, `PreemptiveResource`, or `Container` — the three most
algorithmically interesting primitives (heap ordering, victim scan, cascade). Add one group each
(mirroring `resource_contention`'s shape; for preemptive, alternate priorities so eviction actually
triggers). Needed to gate the F1/P1/P2 changes with data. Also fix the stale "VecDeque" comment
(D2) while in the file.

## T4. `monte_carlo` panic-propagation test exists? — verified gap — **OPEN (low)**

**Model: Sonnet.** `tests/system.rs::monte_carlo_run` covers ordering but no test asserts the
documented panic re-raise contract (worker panics → `resume_unwind` on caller after all joins).
Add one `#[should_panic]` test (and one for the rayon backend under
`--features monte-carlo`, cfg-gated).

---

# Part 6 — Examples

The three examples are a genuine strength — realistic, cross-referenced walkthrough docs, each
exercising a distinct primitive set. Only small issues:

## E1. `hospital`: `ed_cleared_at` silently reports the safety-net deadline — **OPEN (low)**

**Model: Sonnet.** File: `examples/hospital.rs:143-151`.

`arrivals` joins all patients under `any_of![all_patients, env.timeout(SIM_DURATION * 10.0)]`. If
the deadline arm wins (patients stuck on a depleted blood bank), `ed_cleared_at` records `4800.0`
as if the ED cleared — the summary table then averages a sentinel into a real metric. Detect which
arm won (compare `env.now()` to the deadline) and either log "ED NOT cleared (deadline)" and report
the stat as such, or expose it as `Option<f64>` in `SimResult`. Same pattern exists in
`warehouse.rs`'s stream joins — check it too.

## E2. Example-level nits — **OPEN (low)**

**Model: Sonnet.**

- `hospital.rs:191-203`: the eviction scan runs *before* `beds.request()`; if two critical patients
  arrive in one tick both may fire evictions while only one bed frees — harmless (second eviction
  just over-frees) but worth a comment since the `.md` presents it as 1:1.
- `examples/compare.rs` is undocumented in `CLAUDE.md`'s example list and has no companion `.md`
  (it is harness infrastructure, not a showcase — one line in CLAUDE.md saying so is enough).
- Examples log via `Rc<RefCell<Vec<String>>>` and write files at the end — good pattern; consider
  extracting the shared `log_dir()` helper into a tiny `examples/common/` module instead of three
  copies (cosmetic only).

---

# Recommended execution order

| # | Item | Model | Why this order |
|---|------|-------|----------------|
| 1 | **F1** handoff fix + T1/T2 race suite | **Opus** | Only deadlock; blocks trust in everything else; touches the same code as P1/P2 |
| 2 | F2 + F3 + F4 (clock monotonicity batch) | Sonnet | Small, independent, high assert-per-line value |
| 3 | F5 + F6 + D5 (contract hardening batch) | Sonnet | Pure asserts + docs |
| 4 | A4 rand 0.9 decision/upgrade | **Opus** | Public-trait impact; must precede the API freeze |
| 5 | A2 module privacy + A1 crate docs + A3 doc-tests | Sonnet | Breaking surface changes — before 0.1.0 |
| 6 | D4 MSRV correction (PUBLISHING.md) | Sonnet | One wrong number away from a broken release |
| 7 | A5, A6, A7 API polish | Sonnet | Additive; nice before freeze but not blocking |
| 8 | P1 waker caching | **Opus** | After F1 so the executor is touched once, with benches from T3 in place |
| 9 | P2 slab table + T3 benches | Sonnet | Perf, gated by benches |
| 10 | D1-D3 doc rot + E1-E2 examples | Sonnet | Any time; batch into the release-prep PR |

**Gate for every step:** `cargo test` (both feature sets), `cargo clippy --all-targets -- -D warnings`,
and for steps 1, 4, 8, 9 additionally `cargo bench` against a saved baseline and
`compare/run_comparison.sh --no-perf`. Update SPEC.md/API.md/TESTING.md in the same PR as any
behaviour change (per CLAUDE.md), and flip this file's finding statuses as they land.

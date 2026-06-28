# Test Strategy

## Running the tests

```bash
cargo test                   # all tests
cargo test --test timeout    # timeout tests only
cargo test --test event      # event tests only
cargo test --test resource   # resource tests only
cargo test --test preemptive_resource  # preemptive-resource tests only
cargo test --test system     # system tests only
```

## Running the benchmarks

```bash
cargo bench                                    # all benchmark groups
cargo bench -- timeout_throughput              # one group only
cargo bench -- --save-baseline main            # save current results as baseline
cargo bench -- --baseline main                 # compare against saved baseline
cargo bench -- --output-format bencher         # machine-readable output for CI
```

HTML reports are written to `target/criterion/`.

## Checking test coverage

Coverage requires `cargo-llvm-cov` and the `llvm-tools` rustup component:

```bash
# One-time setup
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov

# Summary (one line per file)
cargo llvm-cov --summary-only

# Annotated source (shows hit counts per line)
cargo llvm-cov --text

# HTML report (opens in browser)
cargo llvm-cov --open
```

Current coverage: **~98% lines** across all library source files (86 integration tests + 17 inline unit tests).
(The per-file percentages below were last measured before the `WaitQueue`/`PreemptiveResource`/`rng`
additions; re-run `cargo llvm-cov` to refresh.)

| File | Line coverage |
|---|---|
| `timeout.rs` | 100% |
| `executor/mod.rs` | 100% |
| `executor/waker.rs` | 100% |
| `event.rs` | 100% |
| `resource/mod.rs` | 100% |
| `resource/wait_queue.rs` | covered by inline unit tests |
| `resource/preemptive.rs` | covered by `tests/preemptive_resource.rs` |
| `rng.rs` | covered by inline unit tests + `tests/external_feed.rs` |
| `monte_carlo.rs` | 100% |
| `env.rs` | 99% |
| `executor/queue.rs` | 90% |

The remaining gap in `executor/queue.rs` is the noop-waker vtable callbacks
(`clone`, `wake`, `drop`) inside the `#[cfg(test)]` block — test infrastructure
that is intentionally never invoked.

## Structure

All tests live in `tests/` and use Rust's native integration test harness.
No mocking — every test drives real `SimEnv` instances.

Shared idiom for capturing process output:

```rust
type Log = Rc<RefCell<Vec<String>>>;
fn new_log() -> Log { Rc::new(RefCell::new(Vec::new())) }
```

All tests use `SimEnv::with_seed(0)` (or another fixed seed) for reproducibility.

## Test files

### tests/timeout.rs — 10 tests

| Test | What it verifies |
|---|---|
| `single_timeout_advances_time` | `env.now()` advances correctly after a timeout |
| `multiple_timeouts_fire_in_time_order` | Events fire in time order, not spawn order |
| `run_until_stops_at_boundary` | `run_until(t)` stops at `t`, pending events beyond `t` do not fire |
| `zero_delay_timeout` | `timeout(0.0)` still enters the queue; process completes with `env.now() == 0.0` |
| `deterministic_tie_breaking` | Two processes with equal delay complete in spawn order (via `seq_counter`) |
| `simenv_new_creates_valid_env` | `SimEnv::new()` (entropy-seeded) produces a usable environment |
| `simenv_timeout_method` | `timeout()` called directly on `SimEnv` (not via `EnvHandle`) |
| `rng_guard_covers_all_rngcore_methods` | `next_u32`, `fill_bytes`, `try_fill_bytes` all delegate correctly |
| `timeout_repoll_before_deadline_returns_pending` | Regression: `all_of!` re-polls of a scheduled-but-unfired `Timeout` return `Pending` |
| `any_of_first_pass_all_pending` | `any_of!` first-poll pass does not falsely resolve unfired timeouts |

### tests/event.rs — 5 tests

| Test | What it verifies |
|---|---|
| `basic_fire_and_wake` | Waiter wakes at the correct time when trigger fires |
| `fire_before_await_resolves_immediately` | `fired` latch: awaitable resolves without suspending if already fired |
| `multi_waiter_all_wake` | All waiters are woken simultaneously when trigger fires (`waiters.drain(..)`) |
| `clone_shares_underlying_event` | Cloned `EventAwaitable` shares the same `Rc<RefCell<EventState>>` |
| `envhandle_event_method` | `event()` called directly on `EnvHandle` (not via `SimEnv`) |

### tests/resource.rs — 6 tests

| Test | What it verifies |
|---|---|
| `acquire_when_capacity_available` | Process acquires without suspending when capacity is free |
| `block_and_wake_on_drop` | Waiter unblocks exactly when the guard is dropped |
| `fifo_ordering_three_waiters` | Waiters are served in spawn order (`VecDeque` push_back/pop_front) |
| `guard_drop_releases_exactly_one` | Dropping a guard wakes exactly one waiter, not all |
| `in_use_and_capacity_counters` | `in_use()` and `capacity()` return correct values throughout the lifecycle |
| `zero_capacity_panics` | `Resource::new(0)` panics with the expected message |

### tests/priority_resource.rs — 7 tests

| Test | What it verifies |
|---|---|
| `acquire_immediately` | Free capacity → resolves without suspending |
| `higher_priority_served_first` | Priority 0 waiter woken before priority 1, even if priority 1 spawned first |
| `fifo_within_same_priority` | Three waiters at the same priority level served in spawn order (`seq` counter) |
| `guard_drop_releases_exactly_one` | `BinaryHeap::pop()` wakes only the top-priority waiter, not all |
| `in_use_and_capacity_counters` | `in_use()` and `capacity()` track correctly throughout the lifecycle |
| `multi_capacity_mixed_priorities` | Capacity=2 with four interleaved priorities; on simultaneous release both priority-0 waiters acquire ahead of priority-1 |
| `zero_capacity_panics` | `PriorityResource::new(0)` panics with the expected message |

### tests/preemptive_resource.rs — 11 tests

Covers `PreemptiveResource`: cooperative-at-yield preemption, victim selection,
and capacity accounting under eviction.

| Test | What it verifies |
|---|---|
| `acquire_immediately_when_free` | Free capacity → resolves without suspending or preempting |
| `higher_priority_preempts_holder_immediately` | A higher-priority request evicts a holder at its yield point (t=1, not after the holder's 100-unit service) |
| `equal_priority_does_not_preempt` | An equal-priority request does not preempt; it queues and waits |
| `lower_priority_request_waits_for_release` | A lower-priority request cannot preempt; waits for normal release |
| `preempts_lowest_priority_holder_among_many` | With capacity 2, the priority-8 holder is evicted, not the priority-3 one |
| `tie_break_preempts_most_recently_acquired` | Among equal-lowest-priority holders, the most recently acquired is evicted |
| `preemptor_blocks_when_no_victim_available` | Equal-priority request with no valid victim blocks until release |
| `preempted_guard_drop_does_not_double_release` | Dropping an already-preempted guard is a no-op (no `in_use` underflow / spurious free) |
| `release_wakes_blocked_waiter_in_priority_order` | Plain blocked-waiter path still serves in priority order (delegated to `WaitQueue`) |
| `zero_capacity_panics` | `PreemptiveResource::new(0)` panics with the expected message |
| `woken_waiter_losing_same_tick_race_is_not_starved` | Two equal-priority requests race for one freed unit at the same tick; the loser stays queued and is still served later — neither is starved |

### tests/combinator.rs — 8 tests

#### AnyOf

| Test | Scenario | What it verifies |
|---|---|---|
| `any_of_first_timeout_wins` | `any_of![timeout(1.0), timeout(5.0)]` | Resolves at t=1; earlier future wins |
| `any_of_event_beats_timeout` | `any_of![timeout(10.0), signal]`; event fires at t=2 | Resolves at t=2 even though timeout is pending |
| `any_of_already_fired_event` | Event fired before `any_of` is awaited | Resolves at t=0 via the `fired` latch |
| `any_of_empty_panics` | `AnyOf::new(vec![])` | Panics with expected message |

#### AllOf

| Test | Scenario | What it verifies |
|---|---|---|
| `all_of_waits_for_last` | `all_of![timeout(1.0), timeout(2.0), timeout(5.0)]` | Resolves at t=5 (slowest) |
| `all_of_two_events` | `all_of![event_a, event_b]`; A fires at t=3, B fires at t=7 | Resolves at t=7 |
| `all_of_empty_resolves_immediately` | `AllOf::new(vec![])` | Resolves at t=0 without suspending |
| `all_of_mixed_timeout_and_event` | `all_of![timeout(5.0), signal]`; signal fires at t=8 | Resolves at t=8 |

### tests/container.rs — 15 tests

| Test | What it verifies |
|---|---|
| `get_immediate_when_level_sufficient` | `get` resolves without suspending when level ≥ amount |
| `put_immediate_when_space_available` | `put` resolves without suspending when space available |
| `get_blocks_then_wakes_on_put` | `get` suspends; wakes exactly when a `put` makes level sufficient |
| `put_blocks_then_wakes_on_get` | `put` suspends when full; wakes when a `get` frees space |
| `fifo_ordering_for_get_waiters` | Three blocked `get`s served in spawn order as `put`s trickle in |
| `fifo_ordering_for_put_waiters` | Three blocked `put`s (full container) served in spawn order |
| `fresh_small_get_queues_behind_blocked_large_get` | Strict head-of-line FIFO: a fresh fitting `get` does **not** bypass an already-blocked larger `get` (matches SimPy) |
| `fresh_small_put_queues_behind_blocked_large_put` | Strict head-of-line FIFO: a fresh fitting `put` does **not** bypass an already-blocked larger `put` |
| `cascade_satisfies_multiple_gets` | One large `put` wakes multiple pending `get`s in one cascade pass |
| `cascade_chain_get_put_get_put` | `trigger_cascade` loop resolves a 5-step get→put→get→put→get chain in one pass |
| `immediate_get_cascade_wakes_multiple_blocked_puts` | An immediate `get` frees space and wakes multiple blocked `put`s in one cascade pass (put-side mirror) |
| `level_and_capacity_accessors` | `level()`/`capacity()` return correct values before/after operations |
| `zero_capacity_panics` | `Container::new(0.0, 0.0)` panics |
| `negative_capacity_panics` | `Container::new(-1.0, 0.0)` panics |
| `initial_level_exceeds_capacity_panics` | `Container::new(5.0, 6.0)` panics |

### tests/process_handle.rs — 9 tests

Covers the `ProcessHandle<T>` feature: `spawn` returning a handle, awaiting
it, detaching via drop, composition with combinators, and nesting.

| Test | What it verifies |
|---|---|
| `handle_returns_value` | `spawn(async { ...; T })` returns a handle whose `.await` yields `T` |
| `await_before_completion` | Awaiter suspends until the spawned process finishes |
| `await_after_completion` | If the child finished long ago, awaiting resolves in the same poll (no suspension) |
| `handle_dropped_detaches_process` | Dropping the handle at t=0 does not stop the child; its side-effect runs |
| `completed_unawaited_drops_value` | Handle drop after child completion runs `Drop` on the stored `T` (verified with a drop-counter) |
| `all_of_on_handles` | `all_of![h1.discard(), h2.discard()]` waits for the slowest child |
| `any_of_on_handles` | `any_of![h1.discard(), h2.discard()]` resolves when the first child finishes |
| `nested_handle` | Inner process returns `ProcessHandle<u32>`; outer awaiter unwraps twice |
| `handle_awaited_from_different_process` | A handle can be moved across processes and awaited from any of them |

### tests/dropped_awaitable.rs — 6 tests

Verifies that abandoning a suspendable future (e.g., via `any_of!` where a
competing arm wins) does not corrupt the primitive's internal waiter queue.
All primitives use a shared `Rc<Cell<bool>>` canceled flag between the
request future and its queue entry.

| Test | What it verifies |
|---|---|
| `dropped_event_awaitable_does_not_block_others` | Dropping one `EventAwaitable` does not prevent other waiters from being woken on `fire()` |
| `dropped_resource_request_does_not_starve_followup` | Live waiter behind a canceled entry is still served on guard release |
| `dropped_priority_resource_request_does_not_starve_followup` | Same, for `PriorityResource` |
| `dropped_container_get_does_not_leak_level` | Cascade does not deduct level for a canceled `get` entry |
| `dropped_container_put_does_not_add_level` | Cascade does not add level for a canceled `put` entry |
| `dropped_container_get_does_not_starve_followup` | Live `get` waiter behind a canceled one is still served by a later `put` |

### tests/system.rs — 4 tests

Scenario: **Job Shop with Quality Gate** — a machine (`Resource`, capacity 1) and a
quality gate (`EventTrigger`/`EventAwaitable`). No job may start until the inspector
fires the gate at t=3. Three jobs then queue for the machine sequentially.

| Test | What it verifies |
|---|---|
| `test_system_all_primitives` | Exact event trace with fixed durations: `["gate:3", "job1_done:5", "job2_done:7", "job3_done:9"]`, `env.now() == 9.0` |
| `test_system_determinism` | Same seed → identical trace; different seed → different trace (RNG-driven durations) |
| `monte_carlo_run` | `monte_carlo::run` spawns one thread per seed, returns results in seed order |
| `dropping_env_reclaims_suspended_processes` | Dropping a `SimEnv` with a still-suspended process breaks the `SimState`↔process `Rc` cycle (no leak across replications) |

### tests/external_feed.rs — 5 tests

Scenario: a small M/M/1-style model driven by `SimEnv::with_source(SplitMix64::new(seed))`,
exercising the pluggable external random feed (`src/rng.rs`).

| Test | What it verifies |
|---|---|
| `same_seed_produces_identical_trace` | Two runs with the same seed produce an identical trace (full determinism) |
| `different_seeds_diverge` | Different seeds produce different traces |
| `set_seed_restarts_the_stream` | `SimEnv::set_seed` reseeds the source, restarting its stream |
| `env_feed_matches_standalone_splitmix64` | The env feed and a standalone `SplitMix64` produce the same samples |
| `handles_share_one_feed` | All `EnvHandle` clones draw from one shared feed (draws interleave, not restart) |

### src/rng.rs — 9 inline unit tests

Inline `#[cfg(test)]` module covering the portable feed. The SplitMix64
known-answer table is the **same** one asserted by `compare/models/test_feed.py`,
guarding against cross-language drift.

| Test | What it verifies |
|---|---|
| `splitmix64_known_answer_vectors` | Canonical SplitMix64 outputs for seeds 0 and 42 |
| `uniform01_and_exponential_known_answer` | `uniform01`/`exponential` produce the exact shared-table values |
| `uniform01_in_unit_interval` | `uniform01` stays in `[0, 1)` |
| `next_u32_is_high_bits_of_next_u64` | `next_u32` takes the high 32 bits (matches the Python feed) |
| `same_seed_same_stream` | Two feeds with the same seed produce the same stream |
| `reseed_restarts_stream` | `reseed`/`set_seed` restart the stream |
| `exponential_mean_is_sane` | Sampled exponential mean converges to the target |
| `normal_consumes_two_draws_and_is_centered` | `normal` consumes exactly two uniforms and is centered |
| `stdrng_reseed_is_deterministic` | `RandomSource::reseed` on `StdRng` is deterministic |

### src/executor/queue.rs — 3 inline unit tests

Inline `#[cfg(test)]` module testing `ScheduledWaker`'s `PartialEq` implementation,
which cannot be reached from integration tests (the type is `pub(crate)`).

| Test | What it verifies |
|---|---|
| `partial_eq_same_time_and_seq` | Equal time and seq → equal |
| `partial_eq_different_seq` | Same time, different seq → not equal |
| `partial_eq_different_time` | Different time → not equal |

### src/resource/wait_queue.rs — 5 inline unit tests

Inline `#[cfg(test)]` module exercising the `pub(crate)` `WaitQueue<K>` helper
directly (it is not reachable from integration tests). Uses a safe
`std::task::Wake` recorder to observe wake order.

| Test | What it verifies |
|---|---|
| `try_acquire_respects_capacity` | `try_acquire`/`release` track `in_use` against capacity |
| `fifo_order_for_unit_key` | `WaitQueue<()>` serves waiters in pure insertion (FIFO) order |
| `priority_order_then_fifo_within_level` | `WaitQueue<u32>` serves lowest key first, FIFO within a level |
| `release_skips_canceled_waiter` | A canceled top-priority entry is skipped so the next live waiter is woken |
| `release_with_only_canceled_waiters_wakes_nobody` | All-canceled queue wakes no one but still returns the unit |

## Benchmark groups (benches/simulation.rs)

All benchmarks use `SimEnv::with_seed(0)` — fully deterministic, no file I/O.
N values are the parameterized workload sizes passed to `BenchmarkId`.

| Group | What it measures | N values |
|---|---|---|
| `timeout_throughput` | Raw event-queue + executor throughput (BinaryHeap push/pop, RefCell borrows, waker path) | 1 000 / 10 000 / 100 000 |
| `resource_contention` | `ResourceRequest` waker registration, `VecDeque` push/pop, `ResourceGuard::Drop` wake chain | 100 / 1 000 / 10 000 |
| `event_broadcast` | `EventAwaitable` waker registration and `waiters.drain(..)` dispatch when all N wake simultaneously | 100 / 1 000 / 10 000 |
| `mixed_workload` | End-to-end throughput combining spawn, timeouts, and two resources (nurse cap=1, beds cap=3) | 100 / 1 000 / 10 000 |
| `monte_carlo_scaling` | Parallelism efficiency: K independent copies of `mixed_workload(100)` via `monte_carlo::run` | 1 / 2 / 4 / 8 threads |

## Cross-engine parity (SimPy)

Beyond the `cargo test` suite, the `compare/` harness validates `simu` against
Python's [SimPy](https://simpy.readthedocs.io/) as a reference oracle. Both
engines draw from the **same portable feed** (`SplitMix64` + shared transforms,
re-implemented in `compare/models/_feed.py`), so the order-insensitive queue
models are checked in **exact mode** — per-seed metrics must match within 1e-9
(they land ~1e-15) — while `hospital` stays on the distributional test for its
eviction-handoff ordering exception. Queue models are also checked against
closed-form queueing theory. Performance is measured on two axes: single-thread
engine efficiency, and a **Monte Carlo** benchmark where simu parallelises
independent replications via `monte_carlo::run` (rayon) while SimPy is
GIL-serialised. The harness is what surfaced the `Container` strict-FIFO
divergence (since fixed). Run it with `compare/run_comparison.sh`; see
[`compare/README.md`](compare/README.md) for methodology and
[`compare/REPORT.md`](compare/REPORT.md) for the latest results.

The portable feed's cross-language known-answer test (`compare/models/test_feed.py`)
shares its SplitMix64 vectors with the Rust `rng` unit tests, so the two
implementations cannot silently drift.

Accepted, documented divergences (currently just `hospital.early_discharged`)
are listed in a `KNOWN_EXCEPTIONS` allowlist in `harness/run_correctness.py`:
they are surfaced in the report as `known ⚠` but do not fail the run, so the
harness exits non-zero only on a genuine regression — ready to gate a future CI
job.

## Key implementation notes

- **Sequential process execution within a tick**: the executor runs each process
  to its next suspension point before moving to the next. A guard acquired and
  immediately dropped in one process is gone before the next process runs in the
  same tick. Tests that require two guards to be held simultaneously must yield
  (e.g. via `timeout`) before dropping.
- **Zero-delay timeouts**: `timeout(0.0)` always returns `Pending` on the first
  poll and re-enters the event queue, even though the deadline equals the current
  time. It fires on the next event-loop iteration, not synchronously.
- **RNG sampling before async blocks**: `env.rng()` returns a `RefMut` guard that
  cannot be held across an `.await`. Sample values before the `async move` block
  and move the sampled value in.
- **`Timeout` spurious-wakeup safety**: `Timeout::poll` checks both `scheduled`
  and `env.now() >= deadline` before returning `Ready`. This means `AllOf` (and
  any other combinator) can safely re-poll a `Timeout` sub-future when a *different*
  sub-future fires — the re-polled timeout returns `Pending` until its own deadline
  is reached.

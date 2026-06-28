# simu — Public API Reference

`simu` is a single-threaded discrete-event simulation library inspired by SimPy.
All simulation logic runs inside a `SimEnv`; processes are `async fn`s driven by a custom executor.

---

## Quick start

```rust
use simu::SimEnv;

let mut env = SimEnv::with_seed(42);
let h = env.handle();
env.spawn(async move {
    h.timeout(1.0).await;         // advance simulated time by 1.0
    println!("t={}", h.now());    // prints 1.0
});
env.run();
```

---

## `SimEnv`

```rust
SimEnv::new() -> SimEnv
SimEnv::with_seed(seed: u64) -> SimEnv

env.handle()       -> EnvHandle          // cloneable handle to pass into spawned processes
env.now()          -> f64
env.spawn(future)  -> ProcessHandle<T>   // spawn a root process
env.timeout(delay) -> Timeout            // await to pause for `delay` time units
env.event()        -> (EventTrigger, EventAwaitable)
env.run()                                // run until no more events
env.run_until(t)                         // run until simulated time reaches t
```

`SimEnv` is `!Send + !Sync`. For Monte Carlo, spin up independent instances per OS thread.

---

## `EnvHandle`

Cheap `Clone` (Rc); pass into spawned closures/futures instead of borrowing `SimEnv`.

```rust
handle.now()       -> f64
handle.timeout(delay: f64) -> Timeout
handle.event()     -> (EventTrigger, EventAwaitable)
handle.spawn(future) -> ProcessHandle<T>
handle.rng()       -> impl RngCore + '_   // borrow — do NOT hold across an await point
```

---

## Timeout

```rust
// Future<Output = ()> — resolves after `delay` simulated time units
env.handle().timeout(2.5).await;
```

---

## Events

```rust
let (trigger, awaitable) = env.handle().event();

// In one process:
awaitable.clone().await;          // suspends until trigger fires

// In another process:
trigger.fire();                   // wakes all current and future awaiters (once only)
```

- `EventAwaitable: Clone` — share with multiple processes.
- Fire-before-await latch: awaiting after `fire()` resolves immediately.
- `EventTrigger` can only be fired once (consumes self).

---

## `Resource`

FIFO-queued pool of discrete units.

```rust
use simu::Resource;

let r = Resource::new(capacity);   // panics if capacity == 0; Clone — all clones share pool

r.capacity() -> usize
r.in_use()   -> usize

let guard = r.request().await;     // suspends until a unit is free → ResourceGuard
// unit released automatically when guard is dropped
```

`Resource` is `!Send + !Sync`. Safe to share across processes within one `SimEnv` via `Rc` clone (no `Arc`/`Mutex` needed).

---

## `PriorityResource`

Priority-queued pool; lower number = higher priority; FIFO within equal priority.

```rust
use simu::PriorityResource;

let r = PriorityResource::new(capacity);  // panics if capacity == 0

r.capacity()            -> usize
r.in_use()              -> usize
r.request(priority: u32) -> PriorityResourceRequest  // await → PriorityResourceGuard
```

---

## `PreemptiveResource`

Priority pool whose **in-use** units can be evicted by a higher-priority
request. Like `PriorityResource`, lower number = higher priority and blocked
waiters are served in priority order; *unlike* it, when all units are busy a
higher-priority request preempts the lowest-priority holder that is strictly
worse than itself (ties → most-recently-acquired holder) and takes its unit
immediately.

```rust
use simu::{PreemptiveResource, any_of};

let r = PreemptiveResource::new(capacity);  // panics if capacity == 0

r.capacity()             -> usize
r.in_use()               -> usize
r.request(priority: u32) -> PreemptiveRequest   // await → PreemptiveGuard

// PreemptiveGuard:
guard.preempted()    -> EventAwaitable   // resolves when this unit is preempted
guard.is_preempted() -> bool             // synchronous check
```

Preemption is **cooperative-at-yield**: the executor cannot unwind a suspended
process, so the victim observes preemption at its next `.await` and is expected
to bail. Race your work against the signal:

```rust
let guard = r.request(2).await;
any_of![env.timeout(service_time), guard.preempted()].await;
if guard.is_preempted() {
    return;            // higher-priority work took the unit; clean up and exit
}
// otherwise finished normally; dropping `guard` releases the unit
```

A victim that never checks its signal runs to completion (it has already
surrendered the unit, so it blocks no one). Dropping an already-preempted guard
is a no-op. `PreemptiveResource: Clone` — all clones share the same pool.

---

## `Container`

Continuous quantity with bounded capacity; separate **strict head-of-line FIFO** queues for producers and consumers. A fresh `put`/`get` never jumps ahead of an already-queued waiter, even when the current level would let it complete immediately, so a blocked head-of-queue request holds the line behind it (matching SimPy).

```rust
use simu::Container;

Container::new(capacity: f64, initial_level: f64) -> Container
Container::empty(capacity: f64) -> Container

c.capacity() -> f64
c.level()    -> f64
c.put(amount: f64) -> ContainerPutRequest   // await; suspends if level + amount > capacity
c.get(amount: f64) -> ContainerGetRequest   // await; suspends if level < amount
```

`Container: Clone` — all clones share the same internal state.

---

## `ProcessHandle<T>`

```rust
let handle: ProcessHandle<T> = env.handle().spawn(async { ... });
let result: T = handle.await;    // await the process's return value

// To use with any_of!/all_of!, convert to Future<Output = ()>:
handle.discard().await;          // drops return value
```

Dropping a `ProcessHandle` detaches the process (fire-and-forget); it keeps running.

---

## Combinators

```rust
use simu::{any_of, all_of};

// Resolves when the FIRST future completes; drops the rest
any_of![fut1, fut2, fut3].await;

// Resolves when ALL futures complete
all_of![fut1, fut2, fut3].await;
```

Both macros accept any `Future<Output = ()>`. Use `.discard()` on a `ProcessHandle<T>` to adapt it.

---

## Monte Carlo

`monte_carlo::run` is always available. By default it spawns one `std::thread` per seed. Enabling the
`monte-carlo` feature switches the backend to rayon's bounded thread pool, which scales better for
large seed counts:

`simu = { path = "…", features = ["monte-carlo"] }`

```rust
use simu::monte_carlo;

let results: Vec<R> = monte_carlo::run(seeds, |seed| {
    let mut env = SimEnv::with_seed(seed);
    // build and run simulation
    env.run();
    // return whatever metric you collected
    metric
});
// results are in seed-iteration order; panics in workers re-panic on caller thread
```

---

## Key rules

| Rule | Detail |
|------|--------|
| `!Send + !Sync` | `SimEnv`, `Resource`, `Container` etc. cannot cross thread boundaries; use `monte_carlo::run` for parallelism |
| RNG borrow | `handle.rng()` returns a short-lived guard — sample immediately, do not hold across `.await` |
| RAII guards | `ResourceGuard` / `PriorityResourceGuard` / `PreemptiveGuard` release their unit on drop; drop early to free sooner (a *preempted* `PreemptiveGuard` drop is a no-op) |
| Cancelled requests | Dropping a `request().await` future mid-suspension removes it from the queue |
| Determinism | Same seed + same logic → identical event sequence every run |
| Panics | Programming misuse (wrong capacity, empty combinator, etc.) panics; don't catch them |

# Using SimCore (crates.io: `simcore-des`) — context for AI coding assistants

Drop this file into your project's agent context (CLAUDE.md, cursor rules, etc.)
when working with the SimCore discrete-event simulation kernel. Full reference:
`llms.txt` in the SimCore repository (https://github.com/JetSquirrel/SimCore),
or https://docs.rs/simcore-des (start at the `tutorial` module).

SimCore is a tiny deterministic discrete-event simulation kernel for building
system simulators.

## Setup

- Cargo.toml: `simcore-des = "0.1"` (feature `monte-carlo` for rayon-backed replications).
- Code: `use simcore::{SimEnv, Resource, ...};` — the **package** is `simcore-des`, the
  **library** is `simcore`.

## Core idioms

- A simulation = one `SimEnv` + spawned `async` processes + `env.run()` (or
  `run_until(t)`). Nothing advances until you call run.
- Processes receive a cheap-`Clone` `EnvHandle` (`env.handle()`), giving `now()`,
  `timeout(f64)`, `event()`, `spawn()`, `rng()`.
- Waiting is awaiting: `h.timeout(5.0).await`. Time is `f64`, unit-less, monotonic.
- `SimEnv::with_seed(n)` ⇒ fully deterministic runs (same seed = same results).
- Resources release via RAII: `let _guard = res.request().await;` — the unit frees
  when the guard drops. Always **bind** the guard (a bare `.await;` drops instantly).
- Share resources by **cloning the handle**: `res.clone()` is the same pool.
- Wait for a child process: `let ph = h.spawn(async move { ...; value }); ph.await`.
- Signalling: `let (trigger, awaitable) = env.event();` — `awaitable.clone().await`
  in any number of waiters, one consuming `trigger.fire()`.
- Cancellation (SimPy `Interrupt` does not exist here): race futures —
  `any_of![h.timeout(work), stop_signal].await` — the losing future is dropped.
  For eviction from a resource, use `PreemptiveResource` and race
  `guard.preempted()`, then check `guard.is_preempted()`.
- Stochastic times: `let d = simcore::rng::sample::exponential(&mut h.rng(), mean);`
  **before** any `.await`, then `h.timeout(d).await`.
- N replications in parallel: `monte_carlo::run(0..n, |seed| { build env inside;
  run; return metric })` — results in seed order.

## Type map

| Need | Type |
|------|------|
| FIFO pool of units (servers, spots, machines) | `Resource::new(cap)` → `request().await` |
| Priority queue for units (lower u32 = more urgent) | `PriorityResource::new(cap)` → `request(prio).await` |
| Urgent work evicts running work | `PreemptiveResource::new(cap)` (+ `preempted()` race) |
| Continuous stock (tank, battery, inventory) | `Container::new(cap, initial)` → `put(x)/get(x).await` |
| One-shot broadcast signal | `env.event()` → `EventTrigger` / `EventAwaitable` |
| First-of / all-of | `any_of![...]` / `all_of![...]` (arms must be `Future<Output = ()>`; adapt a `ProcessHandle` with `.discard()`) |

## Hard rules (violations = compile error or panic)

1. **Never** wrap SimCore types in `Arc`/`Mutex` or move them across threads — they are
   `!Send + !Sync`. Parallelism is per-replication only (`monte_carlo::run` builds a
   fresh `SimEnv` inside each closure).
2. **Never** hold `h.rng()` across an `.await`, and never take two rng guards at
   once — runtime panic (`RefCell already borrowed`). Sample into locals first.
3. `fire()` consumes the trigger; an event fires at most once. Late awaiters resolve
   immediately (latch). A dropped, unfired trigger strands its waiters silently.
4. Timeouts panic on negative/non-finite delays — clamp samples from distributions
   that can go negative.
5. `Container::put/get` panic on `amount <= 0` or `amount > capacity`.
6. No tokio/async-std APIs inside processes — SimCore's own executor drives everything.
7. Share mutable state between processes with `Rc<RefCell<...>>`, borrowing only in
   short scopes that contain no `.await`.

## Minimal working example

```rust
use simcore::{SimEnv, Resource};

let mut env = SimEnv::with_seed(42);
let bcs = Resource::new(2); // two charging spots

for i in 0..4u32 {
    let h = env.handle();
    let station = bcs.clone();
    env.spawn(async move {
        h.timeout(f64::from(i) * 2.0).await; // arrive staggered
        let _spot = station.request().await; // FIFO queue
        h.timeout(5.0).await;                // charge
    }); // guard drops → next car gets the spot
}

env.run();
assert_eq!(env.now(), 12.0);
```

## SimPy translation (if porting)

`env.process(f(env))` → `env.spawn(async move { ... })` · `yield env.timeout(5)` →
`h.timeout(5.0).await` · `yield proc` → `handle.await` · `with res.request() as r:
yield r` → `let _g = res.request().await` · `event.succeed()` → `trigger.fire()` ·
`env.run(until=15)` → `env.run_until(15.0)` · `simpy.Interrupt` → `any_of!` race or
`PreemptiveResource`.

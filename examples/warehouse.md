# Warehouse Simulation — Example Walkthrough

> This file walks through `examples/warehouse.rs`, in the same spirit as the
> sibling walkthroughs (`hospital.md`, `brewery.md`). The domain model, resource
> choices, process lifecycles, and feature mapping below describe what the
> implementation does. The "Sample output" section is **illustrative** — its
> numbers show the intended shape of the summary table; exact figures vary by
> seed (run the example to see live values).

## The Warehouse / Distribution-Center Operation

A distribution center (DC) is a building whose entire job is to take goods *in*
from suppliers, hold them briefly, and push them back *out* to customers. Unlike
a factory it transforms almost nothing — its product is **throughput**: the
speed and reliability with which a case of goods travels from an inbound trailer
to an outbound one. Every DC therefore lives or dies by how well it shares a
small fleet of expensive, mobile resources (people and forklifts) across two
competing streams of work — **receiving** and **shipping** — that peak at
different times of day and constantly fight over the same equipment.

The two streams meet at a single shared pool of stock. That coupling — *you
cannot ship what has not yet been put away* — is the heart of this simulation.

### Stage 1 — Inbound: the receiving workflow

Goods arrive on trucks, usually against a pre-booked **dock appointment** (a
scheduled slot) and accompanied by an **ASN** (Advance Ship Notice) that tells
the DC what is on the trailer. The physical sequence is:

1. **Dock assignment.** The truck backs onto one of a limited number of **dock
   doors** — a height-adjustable platform (the *dock leveler*) bridges the gap
   between the trailer bed and the warehouse floor so a forklift can drive
   straight in. North American docks are standardized at roughly 48–52 inches
   high for exactly this reason. Dock doors are scarce, so trucks queue for one.

2. **Unloading.** A **forklift** drives into the trailer and pulls **pallets**
   (standardized load platforms — the GMA 48×40-inch footprint dominates North
   America) onto the dock floor. A fully palletized trailer unloads far faster
   than a hand-stacked ("floor-loaded") one. The truck occupies the dock door
   for the whole of unloading.

3. **Receiving check / QC.** Staff verify the delivered goods against the ASN —
   counts, damage, labels — before the stock is allowed into inventory. Goods
   that pass are "received"; the elapsed time from truck-arrival to
   inventory-availability is the DC's **dock-to-stock** time, a headline KPI.

4. **Putaway.** A forklift (often a **reach truck**, designed for the narrow
   aisles and tall racking of a modern warehouse) carries each pallet from the
   dock to its **storage location** in the racking and books it into stock.
   Putaway is internal, unhurried work: a half-finished putaway can be safely
   **parked** — the pallet simply sits in a staging lane until a driver returns.
   This "parkability" is what makes putaway the natural *preemptible* task.

(Some freight never gets put away at all — in **cross-docking** it moves
straight from an inbound door to an outbound one. Cross-dock buildings are even
shaped for it: Bartholdi & Gue (2004) showed the cost-minimizing footprint is an
elongated **"I"** for facilities up to ~150 doors, a **"T"** up to ~200, and an
**"X"** beyond that. We do not model cross-docking here, but it is why a DC keeps
inbound and outbound doors close together.)

### Stage 2 — Outbound: the order-fulfillment workflow

Customer orders arrive continuously and are released to the floor in **waves** —
batches of orders grouped by a Warehouse Management System (WMS) to smooth the
workload and line up with truck departure times. Waves typically run 1–4 hours,
giving 2–8 waves per shift. Within a wave each order flows through:

1. **Picking.** A **picker** travels to the stock locations and removes the
   ordered quantity of cases. Picking is the single most labor-intensive DC
   activity, so pickers are the scarce human resource. Crucially, an order
   flagged **expedite / "hot"** is allowed to jump ahead of routine orders in
   the pick queue — but a pick already underway is *never* abandoned (you cannot
   un-pick a case that is already in your hand). Picking is therefore
   *priority-scheduled but non-preemptive*.

2. **Packing.** Picked goods go to a **packing station** where they are checked,
   boxed, weighed, and labeled for shipment. Packing stations are a small,
   fixed set of physical benches.

3. **Loading & shipping.** The packed order is moved by **forklift** onto a
   waiting outbound trailer at a dock door. Because a parked trailer is burning
   its **carrier cut-off** clock (miss the cut-off and the order ships a day
   late), loading is *urgent* work — it takes priority over internal putaway and
   will pull a forklift off a parked pallet to get the trailer out on time.

### Why a forklift fleet is the perfect preemption study

The forklift fleet is shared across **three** tasks drawn from both streams:
inbound **unload**, outbound **load**, and internal **putaway**. The first two
involve a *truck physically waiting at a dock* (detention and cut-off clocks
running); the third does not. So when every forklift is busy and a truck-side
job appears, the right operational decision is exactly what `PreemptiveResource`
models: **evict the lowest-priority holder** — a routine putaway — have that
driver *set the pallet down at a safe point*, and send them to the urgent job.
The parked putaway resumes later. No other example in this repository exercises
this primitive, which is the motivation for adding `warehouse.rs`.

### Units of measure

Goods move through a DC in a tiered hierarchy: **pallets** (≈ a trailer's worth
in tens), which break down into **cases**, which break down into **eaches**
(individual selling units), each identified by a **SKU** (Stock Keeping Unit). A
real DC tracks thousands of SKUs. To keep the example focused on *library
features* rather than inventory bookkeeping, this simulation collapses that
catalog into **one generic stock pool measured in cases** — an inbound delivery
adds cases, an outbound order removes them. The mechanics of the `Container`
(and the stockout dynamics it produces) are identical; only the SKU labels are
abstracted away.

---

### Further reading

Authoritative sources used to model the workflows above:

- [Loading dock](https://en.wikipedia.org/wiki/Loading_dock) — dock doors,
  levelers, the 48–52″ standard height.
- [Forklift](https://en.wikipedia.org/wiki/Forklift) — counterbalance vs. reach
  trucks vs. pallet jacks vs. order pickers; warehouse load capacities.
- [Cross-docking](https://en.wikipedia.org/wiki/Cross-docking) — the I/T/X
  facility-shape result (Bartholdi & Gue, *Transportation Science*, 2004).
- [Wave picking](https://en.wikipedia.org/wiki/Wave_picking) — wave planning,
  1–4 h waves, 2–8 waves per shift.
- [Order processing](https://en.wikipedia.org/wiki/Order_processing) and
  [Warehouse management system](https://en.wikipedia.org/wiki/Warehouse_management_system)
  — picking methods and WMS-driven work release.
- [Pallet](https://en.wikipedia.org/wiki/Pallet) and
  [Stock keeping unit](https://en.wikipedia.org/wiki/Stock_keeping_unit) — the
  pallet → case → each hierarchy and SKUs.
- Canonical texts: Bartholdi & Hackman, *Warehouse & Distribution Science*
  (free at warehouse-science.com); Frazelle, *World-Class Warehousing and
  Material Handling*.

> Durations and counts in this simulation are **illustrative** — tuned to
> produce interesting contention on a single screen of log output, not taken as
> exact figures from any one facility. The *workflow structure* is what the
> sources above anchor.

---

`examples/warehouse.rs` is a Monte Carlo distribution-center simulation set in
the **logistics / material-handling** domain. Its centerpiece is a
**preemptible forklift fleet** shared between receiving and shipping, which makes
it the first example to exercise `PreemptiveResource`.

It exercises every public primitive of `simu` in a single scenario:

| Feature                       | How it is used                                                                                          |
|-------------------------------|---------------------------------------------------------------------------------------------------------|
| **`PreemptiveResource`**      | **Forklift fleet** — routine putaway (priority 1) is evicted by truck unload / outbound load (priority 0); the bumped driver parks the pallet and finishes it later |
| `Resource`                    | Dock doors and packing stations (FIFO, capacity-limited)                                                 |
| `PriorityResource`            | Pickers — expedite orders (priority 0) jump the queue ahead of standard (priority 1), but never evict an in-progress pick |
| `Container`                   | On-hand inventory (cases): inbound putaway `put`s stock, outbound picking `get`s it — a stockout suspends pickers until the next putaway lands |
| `any_of!` + `EventAwaitable`  | Putaway races its work against `guard.preempted()` (the guard hands back an `EventAwaitable`)            |
| `ProcessHandle` + `AllOf`     | Each arrival stream collects its spawned handles and joins them at end-of-day to log when the floor clears |
| `monte_carlo::run`            | Ten seeds execute in parallel on separate OS threads                                                     |

Run it:

```bash
cargo run --example warehouse --release
```

Each seed writes a full event log to `target/sim-logs/warehouse_run_00.log` …
`target/sim-logs/warehouse_run_09.log`; the summary table prints to stdout.

---

## Configuration

Time unit: **minutes**. One run models a single ~10-hour operating day.

```rust
const SIM_DURATION:       f64 = 600.0;       // 10-hour day, in minutes

const TRUCK_ARRIVAL_RATE: f64 = 1.0 / 45.0;  // Poisson, ~1 inbound truck every 45 min (~13/day)
const ORDER_ARRIVAL_RATE: f64 = 1.0 / 12.0;  // Poisson, ~1 order every 12 min (~50/day)
const EXPEDITE_PROB:      f64 = 0.20;        // 20 % of orders are expedite / "hot"

// Inbound (receiving)
const UNLOAD_DURATION:    f64 = 30.0;        // palletized trailer, forklift @ PRIO_TRUCK
const QC_DURATION:        f64 = 10.0;        // receiving check vs. ASN (no resource — a timeout)
const PUTAWAY_DURATION:   f64 = 20.0;        // forklift @ PRIO_PUTAWAY — the preemptible task
const CASES_PER_DELIVERY: f64 = 180.0;       // cases added to stock once putaway completes

// Outbound (fulfillment)
const PICK_MEAN:          f64 = 8.0;         // Exp(1/8) travel + pick time
const PACK_DURATION:      f64 = 5.0;         // packing-station time
const LOAD_DURATION:      f64 = 6.0;         // forklift @ PRIO_TRUCK — load onto trailer
const ORDER_CASES_MEAN:   f64 = 40.0;        // Normal(40, 12), clamped ≥ 1
const ORDER_CASES_STD:    f64 = 12.0;

// Inventory (single abstracted stock pool, in cases)
const STOCK_CAPACITY:     f64 = 2000.0;
const STOCK_INITIAL:      f64 = 800.0;       // safety stock — rides out a thin receiving day

// Forklift priorities (PreemptiveResource — lower = higher priority)
const PRIO_TRUCK:         u32 = 0;           // unload / load: a truck is at the dock, clock running
const PRIO_PUTAWAY:       u32 = 1;           // routine putaway: parkable, therefore preemptible

// Picker priorities (PriorityResource — non-preemptive)
const PICK_EXPEDITE:      u32 = 0;
const PICK_STANDARD:      u32 = 1;
```

Resources per run:

| Resource         | Type                  | Capacity | Role                                                              |
|------------------|-----------------------|---------:|------------------------------------------------------------------|
| Dock doors       | `Resource`            | 4        | Shared by inbound trucks (unload) and outbound loading; FIFO     |
| **Forklifts**    | **`PreemptiveResource`** | **2** | **The shared bottleneck**: unload / load / putaway               |
| Pickers          | `PriorityResource`    | 4        | Expedite vs. standard order picking                              |
| Packing stations | `Resource`            | 2        | Pack picked orders                                               |
| Inventory        | `Container`           | 2000     | On-hand stock in cases; starts at 800                           |

The forklift fleet is intentionally small (2) so that truck-side work regularly
collides with putaway and forces preemption. The starting stock is sized to
**absorb a thin receiving day** (an unlucky low Poisson truck draw) while still
being tight enough that mid-day outbound bursts can outrun inbound replenishment
and briefly **stock out**, suspending pickers — the same "tight buffer" tactic
the brewery example uses for its yeast pool.

---

## Processes

Four kinds of async process run inside each `SimEnv`:

| Process            | Count                | Responsibility                                                          |
|--------------------|----------------------|------------------------------------------------------------------------|
| `truck_arrivals`   | 1 (spawner)          | Generates inbound trucks at Poisson inter-arrivals; joins them at end-of-day |
| `order_arrivals`   | 1 (spawner)          | Generates customer orders at Poisson inter-arrivals; joins them at end-of-day |
| `inbound_truck`    | N (one per truck)    | dock → unload → QC → putaway → `inventory.put`                          |
| `order_fulfillment`| M (one per order)    | pick → `inventory.get` → pack → load → ship                            |

Both spawners collect their children's `ProcessHandle`s and, after the order
book closes at `SIM_DURATION`, join them with `AllOf` (guarded by a hard timeout
safety net). The later of the two completion times is the simulated moment the
DC is fully drained — `day_cleared_at`.

---

## Per-flow lifecycles

### Inbound truck

```
dock door ─► forklift(PRIO_TRUCK) ─► [release forklift] ─► [release dock] ─► QC ─► putaway loop ─► inventory.put(+180)
                 (unload, 30m)                                              (10m)   (forklift @ PRIO_PUTAWAY,
                                                                                     20m, PREEMPTIBLE)
```

The dock door is held only for unloading and is released *before* QC and
putaway, so a truck frees its scarce door the moment its pallets are on the
floor. Putaway then runs on the slowest, lowest-priority forklift cycle — and is
the only forklift task that can be interrupted.

### Outbound order

```
picker(EXPEDITE|STANDARD) ─► inventory.get(cases) ─► [release picker] ─► packing station ─► dock door ─► forklift(PRIO_TRUCK) ─► ship
        (pick, ~8m)            (stockout → wait)                            (pack, 5m)        (held during load)  (load, 6m)
```

The `inventory.get` sits *inside* the picker's hold: a picker who arrives at the
face to find it empty waits there (still occupying the picker resource) until an
inbound putaway replenishes stock. That models a real stockout stalling a picker
and is what couples the outbound stream to the inbound one.

---

## The forklift preemption — message sequence chart

This is the scenario the example exists to show. Two forklifts are busy:
forklift **A** is mid-**putaway** (priority 1), forklift **B** is unloading a
truck (priority 0). An outbound order finishes packing and requests a forklift
to **load** its trailer at priority 0. Both forklifts are busy, so the request
**preempts** the lowest-priority holder — forklift A's putaway.

```mermaid
sequenceDiagram
    autonumber
    participant T  as inbound_truck
    participant Fleet as PreemptiveResource<br/>(2 forklifts)
    participant O  as order_fulfillment
    participant Inv as Container<br/>(inventory)

    Note over T: pallets unloaded, dock released, QC done
    T->>Fleet: request(PRIO_PUTAWAY = 1)
    Fleet-->>T: forklift A (guard)
    T->>T: any_of![timeout(20 putaway), A.preempted()]

    Note over O: order picked, packed, dock claimed
    O->>Fleet: request(PRIO_TRUCK = 0)   [load — truck waiting]
    Note over Fleet: both forklifts busy;<br/>lowest strictly-worse holder = A (putaway, prio 1)
    Fleet-->>O: forklift A's unit (transferred immediately)
    Fleet-->>T: fire A.preempted()

    Note over T: any_of! resolves via preempted() branch
    T->>T: is_preempted() == true
    T->>T: remaining -= elapsed; park pallet in staging
    T->>Fleet: request(PRIO_PUTAWAY = 1)   [re-queue to finish later]
    Note over T: suspends — no forklift free, lower priority

    O->>O: timeout(6 load)
    O-->>Fleet: drop(guard) — release forklift
    Note over Fleet: release wakes the highest-priority waiter

    Fleet-->>T: forklift (guard) — putaway resumes
    T->>T: any_of![timeout(remaining), preempted()]
    Note over T: timer wins this time — putaway completes
    T-->>Fleet: drop(guard)
    T->>Inv: put(CASES_PER_DELIVERY = 180)
    Note over Inv: stock now available — may wake a stocked-out picker
```

### Reading the diagram

1. **Putaway starts as the lowest-priority forklift holder** (priority 1). It
   immediately arms the race `any_of![env.timeout(remaining), fork.preempted()]`
   so it can react the instant it is bumped.

2. **The load request (priority 0) preempts it.** `PreemptiveResource` evicts the
   lowest-priority holder that is *strictly worse* than the incoming request.
   Putaway (1) is strictly worse than the load (0), so its forklift unit is
   transferred to the load *immediately* and its `preempted()` signal fires.
   (If two putaways were in progress, the **most-recently-started** one is
   evicted — it has made the least progress, so the least work is wasted.)

3. **Cooperative-at-yield handoff.** The executor cannot forcibly unwind the
   putaway process, so preemption is observed at its next `.await`. The
   `any_of!` resolves via the `preempted()` branch; `is_preempted()` returns
   `true`; the process records how much work it had completed
   (`remaining -= env.now() - start`), conceptually *parks the pallet in a
   staging lane*, and loops back to re-request a forklift at priority 1.

4. **Resumption.** When the urgent load releases its forklift, the release wakes
   the highest-priority waiter. The parked putaway reacquires a forklift and runs
   its *remaining* time. Only when it completes uninterrupted does it
   `inventory.put(...)` — so **preemption delays stock availability**, which can
   lengthen or trigger an outbound stockout elsewhere. That emergent coupling is
   the most interesting behavior in the model.

---

## Key design patterns

### `PreemptiveResource` vs. `PriorityResource` — the teaching contrast

The example deliberately uses **both** priority primitives, side by side, to make
their difference concrete:

| | Pickers (`PriorityResource`) | Forklifts (`PreemptiveResource`) |
|---|---|---|
| Expedite/urgent work jumps the **queue** | ✅ | ✅ |
| Urgent work can **evict an in-progress holder** | ❌ | ✅ |
| Why | You cannot *un-pick* a case already in hand | A driver can *set a pallet down* and come back |

This is the cleanest possible illustration of *why both types exist in the
library*: the choice is driven by whether the in-progress task is safely
**interruptible**, not by how the queue is ordered.

### Pause/resume with remaining-time bookkeeping

Putaway is the only forklift task that races its work against preemption and
loops to finish the remainder:

```rust
// Putaway is the only preemptible forklift task: a parked pallet is safe.
let mut remaining = PUTAWAY_DURATION;
loop {
    let fork = ctx.forklifts.request(PRIO_PUTAWAY).await;   // priority 1
    let start = env.now();
    any_of![env.timeout(remaining), fork.preempted()].await;
    remaining -= env.now() - start;
    if fork.is_preempted() {
        ctx.stats.borrow_mut().putaway_preemptions += 1;
        // Pallet parked in staging; loop to reacquire a forklift later.
        // (Dropping an already-preempted guard is a no-op — the unit is gone.)
        continue;
    }
    break;   // putaway finished uninterrupted; `fork` drops here, freeing the unit
}
ctx.inventory.put(CASES_PER_DELIVERY).await;   // stock becomes available only now
```

`unload` and `load`, by contrast, request the forklift at `PRIO_TRUCK` (0) and
*do not* arm a `preempted()` race — they are never the lowest-priority holder, so
they are never evicted, and a truck-side job always runs to completion once
started. Keeping all three tasks at just **two** priority levels guarantees that
**putaway is the only task that can ever be preempted**, which keeps the model
easy to reason about.

### `Container` couples the two streams

A single `Container` is the only thing inbound and outbound share. Outbound
`get`s block in the container's FIFO queue during a stockout; an inbound
`put` (only ever reached *after* a putaway fully completes) wakes them. Because
preemption pushes putaway completion later, the forklift contention and the
stockout behavior are linked through the inventory — a small fleet can starve
the pick face even when plenty of stock is sitting, half-put-away, in staging.

### Joining two arrival streams with `AllOf`

Each spawner owns its children and joins them independently:

```rust
// inside truck_arrivals / order_arrivals, after the SIM_DURATION cutoff:
if !child_futs.is_empty() {
    let all = AllOf::new(child_futs);                 // Vec<Pin<Box<dyn Future<Output=()>>>>
    any_of![all, env.timeout(SIM_DURATION * 10.0)].await;   // hard-deadline safety net
}
```

`ProcessHandle::discard()` adapts each `ProcessHandle<()>` into a boxable
`Future<Output = ()>`. `day_cleared_at` is the later of the two streams'
completion times.

### Single-threaded conveniences (shared with the other examples)

- **`WarehouseCtx` is cheaply `Clone`d into every process** — `Resource`,
  `PreemptiveResource`, `PriorityResource`, and `Container` are all
  `Rc<RefCell<…>>` inside, so cloning the whole context is fine on one thread.
- **`Rc<RefCell<Vec<String>>>` log** with a uniform `[t={:6.1}] …` line format —
  no mutex, because a `SimEnv` is single-threaded (`!Send + !Sync`).
- **Sample the RNG before any `.await`** — `env.rng()` returns a guard that
  cannot be held across an await point; the compiler enforces it via `!Send`.

---

## Sample output *(illustrative — exact figures vary by seed)*

The summary table follows the shape of `hospital.rs` / `brewery.rs`. The numbers
below are **hand-sketched to show the intended columns**, not a transcript of any
particular run — run `cargo run --example warehouse` for live values:

```
Seed  Trucks  Orders  Expedite  Standard  Preempts  Stockouts  Dock(m)  Fork(m)  Pick(m)  Cleared
---------------------------------------------------------------------------------------------------
   0      13      49        10        37         6          4      3.1      7.8      5.2    641.0
   1      12      52         9        41         9          7      4.0      9.1      6.4    658.5
   ...
mean    12.6    50.2       9.8      39.1       7.1        5.3      3.6      8.3      5.8    649.7
```

- **Trucks / Orders**: inbound trucks and outbound orders accepted in the day.
- **Expedite / Standard**: completed orders per class.
- **Preempts**: how many times a putaway was bumped off a forklift by truck-side
  work (the headline `PreemptiveResource` metric).
- **Stockouts**: orders whose `inventory.get` had to suspend.
- **Dock / Fork / Pick (m)**: mean minutes each unit of work spent *waiting* for
  that resource. The forklift wait shows the 2-truck fleet saturating.
- **Cleared**: simulated minute the last order shipped — always `> SIM_DURATION`,
  because orders that arrive late still need pick + pack + load after the order
  book closes, and any putaway repeatedly preempted finishes last.

Per-run logs (`target/sim-logs/warehouse_run_00.log` …) contain the full
timestamped event trace.

---

## Files referenced

- **`examples/warehouse.rs`** — the simulation source
- **`examples/hospital.rs`** / **`examples/brewery.rs`** — sister examples
- **`src/resource/preemptive.rs`** — `PreemptiveResource` / `PreemptiveGuard` (the star primitive)
- **`src/resource/priority.rs`** — `PriorityResource`
- **`src/resource/mod.rs`** — `Resource` / `ResourceGuard`
- **`src/resource/container.rs`** — `Container` with FIFO put/get cascades
- **`src/combinator.rs`** — `AnyOf` / `AllOf` + `any_of!` / `all_of!` macros
- **`src/event.rs`** — `EventAwaitable` (handed back by `guard.preempted()`)
- **`src/process.rs`** — `ProcessHandle<T>` and `discard()`
- **`src/monte_carlo.rs`** — `monte_carlo::run`

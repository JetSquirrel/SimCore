# Warehouse Visualization Tool — Implementation Plan

> **Status: design document (awaiting approval before implementation).**
>
> This plan describes a playback visualizer for the `warehouse` example. It
> "replays" a single Monte Carlo run as a schematic warehouse floor plan, with
> full transport controls (play/pause/scrub/step/speed) and a seed selector.
> The page is a self-contained, no-build HTML+JS+SVG app fed by a new
> machine-readable **JSONL sidecar** that `examples/warehouse.rs` will emit
> alongside its existing human-readable `.log`.

---

## 1. Goals & non-goals

### Goals

- **Marketing-grade visual.** A clean, recordable, top-down warehouse schematic
  that makes the simulation legible to a non-expert in a few seconds and looks
  good as a GIF on the README / landing page.
- **Faithful playback.** Drive the animation entirely from a real run's event
  stream, so what's on screen is exactly what the simulation did — same
  determinism guarantee as the logs.
- **Interactive *and* recordable.** Full transport controls for live demos;
  a clean "just press play and loop" path for screen recording.
- **Zero build, zero dependencies, zero server.** Double-click `index.html`
  from disk, drag in a `.jsonl`, watch it play. No npm, no bundler, no CDN.
- **Tell the `PreemptiveResource` story.** Foreground the forklift fleet:
  show putaway being *preempted* by truck-side work (a visible flash + a parked
  pallet), and the downstream *stockout* it can cause on the pick face.

### Non-goals

- Not a general-purpose DES visualizer or a `simcore` library feature — it is an
  example-specific companion tool, scoped to the warehouse domain.
- No editing/authoring of simulations in the browser; it is **playback only**.
- No live streaming from a running `SimEnv`; it consumes a finished log file.
  (The data format is designed so live streaming *could* be added later, but
  that is out of scope here.)
- Not packaged/published (no Rust crate, no web deployment); it ships as static
  files under `examples/`.

---

## 2. Decisions locked in (from interview)

| Question | Decision |
|---|---|
| Tech stack | **Self-contained web page** — vanilla JS + SVG, no build step |
| Marketing use | **Both** interactive *and* recordable |
| Machine-readable data | **Add a JSONL sidecar** (`warehouse_run_NN.jsonl`); leave the pretty `.log` untouched |
| Visual style | **Schematic floor plan** (labeled stations, color-coded states, counters/queues) |
| Playback controls | **Full transport**: play/pause, variable speed, scrubbable timeline, step fwd/back, live event ticker |
| Run selection | **Load any one run; switchable** — drag in / pick any of the 10 seed files |
| This session | **Plan doc first** (this file), build after approval |
| Data loading | **Drag-and-drop / file picker** — works from `file://`, no server needed |
| Location & emission | `examples/warehouse-viz/`; warehouse.rs **always** writes the `.jsonl` |
| Schematic scope | **All five resources + KPI panel + preemption FX** |
| JSON in Rust (Q1) | **Hand-rolled tiny serializer** — no `serde`, no new deps |
| Sidecar emission (Q2) | **Always emit**, unconditional, on every run |
| Queue depth (Q3) | **Example-local pending counters** now; public `waiting()` API is an optional later follow-up (no `src/` change in v1) |
| Theme (Q4) | **Neutral industrial** — slate / safety-amber (putaway) / forklift-blue (truck-work) / green tank / red preemption-stockout flash, via CSS variables |
| Demo asset (Q5) | **Commit one modest GIF** (`examples/warehouse-viz/demo.gif`) so the README renders the demo inline |

---

## 3. Architecture overview

```
                      cargo run --example warehouse
                                  │
                 ┌────────────────┴─────────────────┐
                 ▼                                   ▼
   target/sim-logs/warehouse_run_NN.log   target/sim-logs/warehouse_run_NN.jsonl
        (human-readable, unchanged)            (NEW — machine-readable)
                                                     │
                                       drag-and-drop │ (file://, no server)
                                                     ▼
                       examples/warehouse-viz/index.html
                         ├── app.js      (load → parse → model → animate)
                         ├── render.js   (SVG floor-plan draw + update)
                         └── styles.css  (schematic theme, transport bar)
```

Two work-streams:

1. **Rust side (small, surgical):** add a structured event emitter to
   `examples/warehouse.rs` that writes one JSON object per line to a
   `.jsonl` sidecar, in lockstep with the existing log lines. No library
   changes; resources already expose `in_use()` / `capacity()` / `level()`,
   so each event can carry an exact occupancy snapshot.

2. **Web side (the bulk):** a static SVG app that parses the JSONL into an
   ordered event list, builds a per-frame state model, and renders/animates
   the floor plan under transport control.

---

## 4. Data contract — the JSONL sidecar

### 4.1 Why JSONL (not one big JSON array)

- **Append-friendly & streamable:** one event per line; the file is valid after
  every line, and a future live-tail mode "just works".
- **Trivial to emit from Rust:** `writeln!(f, "{}", json_line)` per event, built
  with a tiny hand-rolled serializer (see §6.3) — **no `serde` dependency added**
  to the example.
- **Trivial to parse in JS:** `text.split('\n').filter(Boolean).map(JSON.parse)`.

### 4.2 File

`target/sim-logs/warehouse_run_NN.jsonl`, one per seed, written next to the
existing `warehouse_run_NN.log`. (Stays under `/target`, so it remains gitignored
and never pollutes the repo.)

### 4.3 Line types

Every line is a JSON object with a `"type"` field. Two non-event header lines come
first, then a time-ordered stream of events.

**(a) `meta` — first line.** Static run facts the viewer needs to lay out the
scene and label the KPI panel.

```json
{"type":"meta","schema":1,"seed":0,"sim_duration":600.0,
 "resources":{
   "dock_doors":{"kind":"Resource","capacity":4},
   "forklifts":{"kind":"PreemptiveResource","capacity":2},
   "pickers":{"kind":"PriorityResource","capacity":4},
   "packing":{"kind":"Resource","capacity":2},
   "inventory":{"kind":"Container","capacity":2000.0,"initial":800.0}
 },
 "constants":{"unload":30.0,"qc":10.0,"putaway":20.0,"cases_per_delivery":180.0,
              "pick_mean":8.0,"pack":5.0,"load":6.0,"expedite_prob":0.20}}
```

**(b) `event` — the stream.** One per state transition, carrying the simulated
time, a semantic event name, the actor, and a **resource-occupancy snapshot taken
at that instant**. Snapshots make the viewer dumb-simple: it never has to *infer*
counts, only display them, and scrubbing to any event shows exactly correct state.

```json
{"type":"event","t":55.4,"ev":"truck_unload_start","actor":{"kind":"truck","id":1},
 "info":{"dock_wait":0.0,"fork_wait":0.0},
 "snap":{"dock_doors":{"in_use":1,"queue":0},
         "forklifts":{"in_use":1,"queue":0,"putaway_active":0},
         "pickers":{"in_use":2,"queue":0},
         "packing":{"in_use":0,"queue":0},
         "inventory":{"level":442.0}}}
```

### 4.4 Event vocabulary (`ev`)

Derived directly from the existing log points in `warehouse.rs`, plus a few extra
transitions the text log glosses over (resource *acquire* vs *start*, needed for
smooth animation). Grouped by actor:

| `ev` | Emitted where (warehouse.rs) | Drives on screen |
|---|---|---|
| `truck_arrive` | "Truck N arrives at the yard" | spawn a truck token at the yard |
| `truck_dock_acquire` | after `dock_doors.request().await` | move truck onto a dock door slot |
| `truck_unload_start` | "Truck N unloading" | mark dock door + 1 forklift busy (unload) |
| `truck_unload_end` | after `timeout(UNLOAD)`, on `drop(fork/dock)` | free dock + forklift |
| `truck_qc_start` / `truck_qc_end` | around `timeout(QC)` | QC badge on the truck |
| `putaway_acquire` | after `forklifts.request(PRIO_PUTAWAY).await` | forklift turns "putaway" colored |
| `putaway_preempted` | the `is_preempted()` branch | **preemption flash** + parked-pallet token; `remaining` shown |
| `putaway_resume` | re-acquire after a preemption | parked pallet picked back up |
| `putaway_done` | "putaway done +180 cases" | `inventory.put`; tank rises; +cases ping |
| `order_arrive` | "Order N received … [class]" | spawn an order token; cases + expedite flag |
| `pick_acquire` | after `pickers.request().await` | order moves to a picker; expedite = distinct color |
| `pick_start` / `pick_end` | around `timeout(pick_dur)` | picker busy bar |
| `stockout_begin` | `inventory.get` suspends (`now>stock_t0`) | order token turns "stalled"; tank empty flash |
| `stockout_end` | the suspended `get` completes | order resumes; tank dips |
| `pack_start` / `pack_end` | around `timeout(PACK)` | packing station busy |
| `load_start` / `load_end` | around `timeout(LOAD)` | dock + forklift busy (load, prio 0) |
| `order_ship` | "Order N shipped" | order token exits; KPI completed++ |
| `gate_closed` | "Inbound gate closed" | stop spawning trucks |
| `book_closed` | "Order book closed" | stop spawning orders |
| `day_cleared` | end of each arrival stream | floor-drained marker / end of timeline |

> Not every listed transition must ship in v1 — the **bold** ones
> (`*_unload_*`, `putaway_*`, `stockout_*`, `order_ship`) are the load-bearing
> story beats. `*_start`/`*_end` pairs that only toggle a busy bar can be added
> incrementally; the `snap` on each event means even a partial vocabulary renders
> correct *counts*, just with less granular token motion.

### 4.5 Snapshot semantics

- `in_use` / `queue` come straight from `resource.in_use()` and a waiter count.
  - `in_use()` is public on `Resource`/`PriorityResource`/`PreemptiveResource`.
  - `queue` (number waiting) is **not currently public**. Options, in order of
    preference: **(a)** the example tracks its own pending counters (increment on
    request-issued, decrement on acquired) — zero library change, fully accurate;
    **(b)** add `pub fn waiting(&self) -> usize` to the resources later. v1 uses
    **(a)** (decided in Q3) to keep the library untouched; (b) is an optional
    later follow-up.
  - `putaway_active` (forklifts busy specifically on putaway) likewise tracked by
    the example, to color forklifts by task.
- `inventory.level` from `Container::level()`.
- Snapshots are emitted **after** the state change the event describes, so frame
  N's `snap` is the world as of just after event N.

### 4.6 Determinism

The JSONL is produced in the same single-threaded run as the `.log`, so it
inherits the example's determinism: same seed → byte-identical `.jsonl`. The
viewer is a pure function of the file, so a recording is reproducible.

---

## 5. The visualization (web app)

### 5.1 Files (`examples/warehouse-viz/`)

| File | Responsibility | Approx size |
|---|---|---|
| `index.html` | Page skeleton: SVG stage, transport bar, KPI panel, event ticker, drop zone | small |
| `styles.css` | Schematic theme (warehouse palette), responsive layout, control styling | small–med |
| `app.js` | Load/parse JSONL, build frame model, transport/clock state machine, wire UI | medium |
| `render.js` | Build the static SVG scene once; update node states per frame | medium |
| `sample-data.md` | How to generate a `.jsonl` and a note pointing back to this plan | tiny |

No external libraries. SVG (not Canvas) so shapes are inspectable, crisp when
recorded/zoomed, and easy to style via CSS.

### 5.2 Floor-plan layout (schematic)

Top-down, left = inbound, right = outbound, mirroring the real I-shaped DC and the
`warehouse.md` narrative (goods flow left→right):

```
┌──────────────────────────────────────────────────────────────────────┐
│  YARD            RECEIVING            STORAGE           SHIPPING        │
│ ┌──────┐    ┌───────────────┐   ┌──────────────┐   ┌───────────────┐   │
│ │trucks│    │ Dock doors x4 │   │  INVENTORY    │  │ Packing x2     │   │
│ │ queue│──▶ │ ▢ ▢ ▢ ▢       │   │  ┌────────┐  │  │ ▢ ▢            │   │
│ └──────┘    └───────────────┘   │  │████████│  │  └───────────────┘   │
│             ┌───────────────┐   │  │████████│  │  ┌───────────────┐   │
│             │ FORKLIFTS x2  │   │  │██ tank █│  │  │ Pickers x4     │   │
│             │ 🚜  🚜        │◀──┼─▶│ 800/2000│  │  │ ▢ ▢ ▢ ▢        │   │
│             │ unload/load/  │   │  └────────┘  │  └───────────────┘   │
│             │ putaway       │   │  staging:▦▦  │   ▲ order queue ◀──── │
│             └───────────────┘   └──────────────┘                       │
│  ◀════ forklifts are shared between receiving & shipping ════▶          │
└──────────────────────────────────────────────────────────────────────┘
  KPI:  t=312.4  Trucks 4/13  Orders 28  Shipped 25  Preempts 6  Stockouts 4
  Ticker: [t=312.4] Order 28 picked through stockout (picker stalled 4.2) …
```

Each **resource** is a labeled group of capacity-many slots:

- **Empty slot** = outlined box; **busy slot** = filled, colored by task.
- **Forklifts** are the centerpiece: a forklift glyph per unit, colored
  **blue = unload/load (PRIO_TRUCK)** vs **amber = putaway (PRIO_PUTAWAY)**;
  on `putaway_preempted` the unit flashes red and a small **parked-pallet** glyph
  appears in a "staging" lane, then is reclaimed on `putaway_resume`.
- **Inventory** is a vertical **tank/bar** filling to `level/capacity`, with the
  numeric `level/2000`. It pulses green on `put`, red when it hits 0 (stockout).
- **Queues** (trucks waiting for a dock, orders waiting for a picker) render as
  small stacked tokens with a count badge.
- **Tokens** (trucks, orders) carry their id and key attribute (cases, expedite).
  Expedite orders are visually distinct (e.g. a "hot" outline) to show them
  jumping the picker queue.

### 5.3 KPI panel (top-right)

Live running totals, recomputed per frame from the latest event/snapshot:
`t`, trucks received / expected, orders received, orders shipped, **preemptions**,
**stockouts**, forklift utilization (in_use/capacity), inventory level. These are
the same headline numbers as the stdout summary table, so the video and the table
agree.

### 5.4 Event ticker

A scrolling list of the last ~8 human-readable lines (reuse the `.log` phrasing,
reconstructed from `ev`+`info`), the current one highlighted. Gives the recording
a "subtitle track" and ties the visual back to the familiar log.

### 5.5 Transport controls (bottom bar)

- **Play / Pause** toggle.
- **Speed**: discrete multipliers (e.g. 0.25× / 0.5× / 1× / 2× / 4× / 8×) mapping
  *simulated minutes → wall-clock seconds*. 1× defined as a sensible default
  (e.g. 1 sim-minute = 100 ms, ~60 s for a 600-min day; tuned during build).
- **Scrubber**: a range slider over `[0, day_cleared_at]`; dragging seeks. Because
  every event carries a full `snap`, seeking = "find last event ≤ t, apply its
  snapshot, interpolate token motion" — cheap and exact.
- **Step ◀ / ▶**: jump to previous/next event (great for explaining a preemption
  frame-by-frame in a demo).
- **Seed selector**: dropdown listing loaded runs; switching reloads the model.
  Drag-and-drop or file-picker adds runs (multi-select supported → populates the
  dropdown). Remembers nothing across reloads (stateless, by design).
- **Loop** toggle: auto-restart at end — the "record a clean GIF" path.

### 5.6 Clock / animation model

- A single `requestAnimationFrame` loop advances a `simClock` by
  `dt_wall * speed * MINUTES_PER_SECOND`.
- Maintain a cursor into the time-sorted event array; apply every event whose
  `t ≤ simClock` since the last frame (updating the authoritative state from each
  `snap`), then render.
- **Token motion** (truck sliding to a dock, order moving to a picker) is
  interpolated *between* the event that starts the move and its end event, using
  simulated time — so motion speed is consistent with the clock and looks right at
  any playback speed.
- Reduced-motion-friendly: if `prefers-reduced-motion`, tokens snap instead of
  glide.

### 5.7 Recording-friendliness

- A `?chrome=off` URL flag (or a toolbar toggle) hides everything but the stage +
  KPI for a clean capture.
- Fixed 16:9 stage viewBox so the SVG scales without reflowing.
- `Loop` + autostart via `?autoplay=1&loop=1&speed=2` so a recording is one click.

---

## 6. Rust changes to `examples/warehouse.rs`

Small and additive. The existing text log and summary table are **untouched**;
we add a parallel structured stream.

### 6.1 A `JsonlEvent` emitter in the context

- Add an `events: Rc<RefCell<Vec<String>>>` (pre-serialized JSONL lines) to
  `WarehouseCtx`, alongside the existing `log`. Or a small `Recorder` struct
  wrapping it plus the pending/putaway counters (§4.5a). Cloned like the rest of
  the context (`Rc` inside).
- A helper `ctx.emit(t, ev, actor, info)` that snapshots the resources
  (`in_use()`, the example-tracked queue counters, `inventory.level()`) and pushes
  one serialized line. Called right next to each existing `log.borrow_mut().push`,
  plus at the extra acquire/start/end transitions in §4.4.

### 6.2 Pending-/putaway-counter bookkeeping (for `queue` + `putaway_active`)

- Increment a `dock_pending` (etc.) counter immediately before each
  `…request().await`, decrement immediately after it resolves; the snapshot reads
  these for `queue`. Same pattern gives `putaway_active` for forklift coloring.
- Pure example-level bookkeeping — **no `simcore` library change in v1**.

### 6.3 Tiny JSON serializer (no `serde`)

- A ~30-line module writing objects/strings/numbers with proper escaping. The
  payload shapes are fixed and simple (strings, f64, ints, small nested objects),
  so a hand-rolled writer is sufficient and keeps `Cargo.toml` dependency-free.
  *(Decided in Q1: hand-roll, no `serde`.)*

### 6.4 Writing the file

- In `run_simulation`, after `env.run()`, write `warehouse_run_NN.jsonl` next to
  the `.log` using the existing `log_dir()` helper. `meta` line first
  (constants/resource capacities), then the recorded event lines.

### 6.5 Determinism & cost

- Recording is O(events) extra `String` pushes; negligible vs. the sim. No change
  to RNG draw order (snapshots only *read* state), so the `.log` stays
  byte-identical — I'll verify seed-3 `.log` is unchanged before/after.

---

## 7. File / repo layout

```
examples/
├── warehouse.rs                 # + structured JSONL emitter (additive)
├── warehouse.md                 # + short "Visualizing a run" section linking the viewer
└── warehouse-viz/
    ├── PLAN.md                  # this document
    ├── index.html               # the app
    ├── styles.css
    ├── app.js
    ├── render.js
    └── sample-data.md           # how to produce a .jsonl + usage
```

Docs to update on build (kept in sync per repo convention):

- `examples/warehouse.md` — add a "Visualizing a run" section (how to generate the
  `.jsonl`, open the page, drag it in; an embedded GIF later).
- `README.md` — one line under Examples pointing at the viewer (marketing entry).
- `CLAUDE.md` — note the new `examples/warehouse-viz/` tool + the `.jsonl` sidecar
  in the source-layout / examples notes.
- `SPEC.md` §7.3 — a sentence that the warehouse example now ships a playback
  visualizer and emits a machine-readable sidecar.

No changes to `src/`, tests, or `Cargo.toml` in v1 (hand-rolled JSON, no `serde`;
queue depth via example counters — see §10).

---

## 8. Implementation phases (after approval)

1. **Data contract.** Finalize `meta` + `event` schemas (this doc), freeze `ev`
   vocabulary and `snap` shape.
2. **Rust emitter.** Add recorder + counters + JSON writer to `warehouse.rs`;
   emit `.jsonl`; verify `.log` byte-identical and `.jsonl` parses; clippy clean
   (both feature sets), `cargo test` still 87 green.
3. **Static scene.** `index.html` + `styles.css` + `render.js`: draw the empty
   floor plan from a `meta` line (no animation yet).
4. **Loader + model.** Drag-and-drop / file-picker → parse → frame model → seed
   dropdown.
5. **Transport + clock.** Play/pause/speed/scrub/step/loop; rAF clock; apply
   snapshots; KPI panel + ticker.
6. **Token motion + FX.** Truck/order glides, forklift task coloring, **preemption
   flash + parked pallet**, inventory tank, stockout flash.
7. **Recording polish.** `?chrome=off`/autoplay/loop, 16:9 viewBox, reduced-motion.
8. **Docs + GIF.** Generate a GIF from a representative seed, embed in
   `warehouse.md` / `README.md`; update `CLAUDE.md` / `SPEC.md`.
9. **Verify & commit.** Cross-browser sanity (Chrome/Firefox from `file://`),
   determinism re-check, then commit (and offer to push).

Phases 2 and 3–4 are independent and could proceed in parallel; everything after
phase 5 is incremental polish that degrades gracefully if time-boxed.

---

## 9. Risks & mitigations

| Risk | Mitigation |
|---|---|
| `file://` can't fetch sibling files for the seed dropdown | Chosen design avoids fetch entirely: drag-and-drop / file-picker. Auto-fetch is an optional later add for users who run a local server. |
| `queue` depth not exposed by the library | Track pending counters in the example (no lib change). Optionally add `waiting()` later (Q3). |
| Hand-rolled JSON escaping bugs | Keep payloads to simple types; unit-free; small writer with explicit string escaping; validate `.jsonl` parses in JS during phase 2. |
| Snapshot bloat (one per event) inflates file size | Logs are ~10 KB text today; JSONL is larger but still tiny (tens of KB) and gitignored under `/target`. Acceptable. |
| Animation looks wrong at high speed | Interpolate token motion in *simulated* time, not frames; cap concurrent animations; snap on reduced-motion. |
| Scope creep on visual fidelity | v1 ships schematic boxes/glyphs; sprites/illustration are explicitly a later option, not v1. |

---

## 10. Resolved decisions (interview round 2)

All five open questions are now settled:

1. **JSON in Rust → hand-roll.** A ~30-line serializer in the example, no `serde`,
   no new `Cargo.toml` dependency. Payload is trivial (strings, f64, ints, small
   nested objects) with explicit string escaping; validated by parsing the
   `.jsonl` in JS during phase 2.
2. **Sidecar emission → always on.** Every `cargo run --example warehouse` writes
   10 `warehouse_run_NN.jsonl` files under `target/sim-logs/` (gitignored). No
   flag, no env var — the viewer is always one drag away.
3. **Queue depth → example-local counters now.** v1 tracks pending/`waiting`
   counts inside the example (no `src/` change). A public
   `waiting()`/`queue_len()` on the resources is recorded as an **optional later
   follow-up**, not part of this work.
4. **Theme → neutral industrial.** Slate background; **safety-amber = putaway
   (PRIO_PUTAWAY)**; **forklift-blue = truck-work (unload/load, PRIO_TRUCK)**;
   green inventory tank; **red flash for preemption and for stockout**. All colors
   exposed as CSS custom properties so the look is restyleable in one place.
5. **Demo asset → commit one GIF.** A single modestly-sized
   `examples/warehouse-viz/demo.gif`, recorded from a representative seed, is
   committed so the README/walkthrough render the demo inline.

No `src/`, test, or `Cargo.toml` changes in v1.
```


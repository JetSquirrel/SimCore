// SPDX-FileCopyrightText: 2026 Christoph Kuhmuench <christoph.kuhmuench@gmail.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

/* ===================================================================== *
 * app.js — load a warehouse JSONL run, reconstruct per-frame state from
 * the event stream, and drive the SVG Scene under full transport control.
 *
 * No frameworks, no modules, no network — works straight from file://.
 *
 * Model: each event marks one actor's state transition. We fold the event
 * stream into a "world" of truck/order entities, then bucket those entities
 * by location into the slot/queue arrays the renderer paints. Inventory
 * level + waiting come straight from each event's authoritative `snap`shot.
 * Seeking backward replays from t=0 (cheap: a few hundred events); seeking
 * forward applies incrementally.
 * ===================================================================== */

(function () {
  "use strict";

  // 1x playback: 10 simulated minutes per wall-clock second (~67 s for a
  // ~675-minute day). Speed multipliers scale this.
  const BASE_RATE = 10;

  const $ = (id) => document.getElementById(id);

  const dom = {
    svg: $("stage"),
    dropzone: $("dropzone"),
    fileInput: $("file-input"),
    pickBtn: $("pick-btn"),
    loadMore: $("load-more"),
    play: $("btn-play"),
    stepBack: $("btn-step-back"),
    stepFwd: $("btn-step-fwd"),
    scrubber: $("scrubber"),
    clock: $("clock"),
    speed: $("speed"),
    loop: $("loop"),
    seedSelect: $("seed-select"),
    ticker: $("ticker"),
  };

  const scene = new Scene(dom.svg);

  /** All loaded runs, keyed by a label (seed or filename). */
  const runs = new Map();

  /** The currently-selected run + replay cursor. */
  let cur = null; // { meta, events, totals, label }
  let world = null;
  let cursor = 0; // index of next event to apply
  let simClock = 0;
  let tEnd = 1;
  let playing = false;
  let lastNow = 0;
  let raf = null;
  let scrubbing = false;
  const tickerLines = [];

  /* =================================================================== *
   * Loading & parsing
   * =================================================================== */

  function parseJsonl(text, fallbackLabel) {
    const lines = text.split("\n");
    let meta = null;
    const events = [];
    for (const raw of lines) {
      const line = raw.trim();
      if (!line) continue;
      let obj;
      try {
        obj = JSON.parse(line);
      } catch (e) {
        continue; // skip malformed lines defensively
      }
      if (obj.type === "meta") meta = obj;
      else if (obj.type === "event") events.push(obj);
    }
    if (!meta) {
      // Tolerate a meta-less file: synthesize a default from known layout.
      meta = {
        schema: 1,
        seed: fallbackLabel,
        sim_duration: 600,
        resources: {
          dock_doors: { kind: "Resource", capacity: 4 },
          forklifts: { kind: "PreemptiveResource", capacity: 2 },
          pickers: { kind: "PriorityResource", capacity: 4 },
          packing: { kind: "Resource", capacity: 2 },
          inventory: { kind: "Container", capacity: 2000, initial: 800 },
        },
      };
    }
    if (events.length === 0) return null;

    // Pre-scan totals for the KPI progress fields.
    let trucksTotal = 0,
      ordersTotal = 0;
    for (const e of events) {
      if (e.ev === "truck_arrive") trucksTotal++;
      else if (e.ev === "order_arrive") ordersTotal++;
    }
    const label =
      meta.seed !== undefined && meta.seed !== null && meta.seed !== ""
        ? "seed " + meta.seed
        : fallbackLabel;
    return {
      meta,
      events,
      label,
      totals: { trucks: trucksTotal, orders: ordersTotal },
    };
  }

  function loadFiles(fileList) {
    const files = Array.from(fileList).filter((f) => /\.jsonl$|\.json$/i.test(f.name) || true);
    let pending = files.length;
    if (pending === 0) return;
    let firstLabel = null;
    files.forEach((file) => {
      const reader = new FileReader();
      reader.onload = () => {
        const run = parseJsonl(String(reader.result), file.name.replace(/\.[^.]+$/, ""));
        if (run) {
          runs.set(run.label, run);
          if (firstLabel === null) firstLabel = run.label;
        }
        if (--pending === 0) {
          rebuildSeedSelect();
          // Select the first newly-loaded run (or keep current if already playing).
          if (!cur && firstLabel) selectRun(firstLabel);
          else if (firstLabel) selectRun(firstLabel);
        }
      };
      reader.onerror = () => {
        if (--pending === 0) rebuildSeedSelect();
      };
      reader.readAsText(file);
    });
  }

  function rebuildSeedSelect() {
    const sel = dom.seedSelect;
    sel.innerHTML = "";
    const labels = Array.from(runs.keys()).sort(naturalCompare);
    for (const label of labels) {
      const opt = document.createElement("option");
      opt.value = label;
      opt.textContent = label;
      sel.appendChild(opt);
    }
    sel.disabled = labels.length === 0;
  }

  function naturalCompare(a, b) {
    return a.localeCompare(b, undefined, { numeric: true });
  }

  /* =================================================================== *
   * World model — fold events into truck/order entities
   * =================================================================== */

  function freshWorld() {
    return {
      t: 0,
      trucks: new Map(), // id -> {state, remaining, cases}
      orders: new Map(), // id -> {state, cases, exp}
      inv: { level: cur.meta.resources.inventory.initial, waiting: 0 },
      kpi: { trucks: 0, orders: 0, shipped: 0, preempts: 0, stockouts: 0 },
      lastNotable: "",
    };
  }

  /** Apply one event to `world`. Returns the event's task-flash target, if any. */
  function applyEvent(w, e) {
    const a = e.actor || {};
    const id = a.id;
    const info = e.info || {};
    w.t = e.t;
    let flash = null;

    switch (e.ev) {
      case "truck_arrive":
        w.trucks.set(id, { state: "yard" });
        w.kpi.trucks++;
        break;
      case "truck_dock_acquire":
        setTruck(w, id, { state: "dockwait" });
        break;
      case "truck_unload_start":
        setTruck(w, id, { state: "unload" });
        break;
      case "truck_unload_end":
        setTruck(w, id, { state: "qc" });
        break;
      case "truck_qc_end":
        setTruck(w, id, { state: "qcwait" });
        break;
      case "putaway_acquire":
      case "putaway_resume":
        setTruck(w, id, { state: "putaway", remaining: info.remaining });
        break;
      case "putaway_preempted":
        setTruck(w, id, { state: "parked", remaining: info.remaining });
        w.kpi.preempts++;
        flash = "fork";
        break;
      case "putaway_done":
        w.trucks.delete(id);
        break;
      case "order_arrive":
        w.orders.set(id, { state: "queued", cases: info.cases, exp: !!info.expedite });
        w.kpi.orders++;
        break;
      case "pick_start":
        setOrder(w, id, { state: "picking" });
        break;
      case "stockout_begin":
        setOrder(w, id, { state: "stalled" });
        w.kpi.stockouts++;
        flash = "inv";
        break;
      case "stockout_end":
        setOrder(w, id, { state: "picking" });
        break;
      case "pick_end":
        setOrder(w, id, { state: "transit" });
        break;
      case "pack_start":
        setOrder(w, id, { state: "packing" });
        break;
      case "pack_end":
        setOrder(w, id, { state: "transit2" });
        break;
      case "order_dock_acquire":
        setOrder(w, id, { state: "loadwait" });
        break;
      case "load_start":
        setOrder(w, id, { state: "loading" });
        break;
      case "order_ship":
        w.orders.delete(id);
        w.kpi.shipped++;
        break;
      case "gate_closed":
      case "book_closed":
      case "day_cleared":
        break;
    }

    // Inventory comes straight from the authoritative snapshot.
    if (e.snap && e.snap.inventory) {
      w.inv.level = e.snap.inventory.level;
      w.inv.waiting = e.snap.inventory.waiting || 0;
    }

    const notable = describe(e);
    if (notable) w.lastNotable = notable;
    return flash;
  }

  function setTruck(w, id, patch) {
    const t = w.trucks.get(id) || {};
    w.trucks.set(id, Object.assign(t, patch));
  }
  function setOrder(w, id, patch) {
    const o = w.orders.get(id) || {};
    w.orders.set(id, Object.assign(o, patch));
  }

  /** Rebuild the world from scratch up to (but not including) event `idx`. */
  function rebuildTo(idx) {
    world = freshWorld();
    for (let i = 0; i < idx; i++) applyEvent(world, cur.events[i]);
    cursor = idx;
  }

  /** Advance the cursor to `idx`, flashing along the way if `live`. */
  function advanceTo(idx, live) {
    if (idx < cursor) {
      rebuildTo(idx);
      return;
    }
    while (cursor < idx) {
      const flash = applyEvent(world, cur.events[cursor]);
      pushTicker(cur.events[cursor]);
      cursor++;
      if (live && flash) scene.flash(flash);
    }
  }

  /* =================================================================== *
   * Derive the renderer's state object from the world
   * =================================================================== */

  function renderState() {
    const w = world;
    const dockSlots = [];
    const forkSlots = [];
    const pickSlots = [];
    const packSlots = [];
    const parked = [];
    const yard = [];

    // Trucks
    const truckIds = Array.from(w.trucks.keys()).sort((x, y) => x - y);
    for (const id of truckIds) {
      const t = w.trucks.get(id);
      if (t.state === "yard") yard.push(id);
      else if (t.state === "dockwait") {
        // Holding a dock, still waiting for a forklift (fleet saturated).
        dockSlots.push({ label: "T" + id, task: "unload" });
      } else if (t.state === "unload") {
        dockSlots.push({ label: "T" + id, task: "unload" });
        forkSlots.push({ label: "T" + id, task: "unload" });
      } else if (t.state === "putaway") {
        forkSlots.push({ label: "T" + id, task: "putaway" });
      } else if (t.state === "parked") {
        parked.push({ id, remaining: t.remaining });
      }
      // 'qc' / 'qcwait' trucks are inside the building, not in a slot.
    }

    // Orders
    const orderIds = Array.from(w.orders.keys()).sort((x, y) => x - y);
    const orderQueue = [];
    for (const id of orderIds) {
      const o = w.orders.get(id);
      if (o.state === "queued") orderQueue.push({ id, cases: o.cases, exp: o.exp });
      else if (o.state === "picking" || o.state === "stalled") {
        pickSlots.push({ label: "O" + id, exp: o.exp, stalled: o.state === "stalled" });
      }       else if (o.state === "packing") {
        packSlots.push({ label: "O" + id, task: "pack" });
      } else if (o.state === "loadwait") {
        // Holding a dock, still waiting for a forklift to load.
        dockSlots.push({ label: "O" + id, task: "load" });
      } else if (o.state === "loading") {
        dockSlots.push({ label: "O" + id, task: "load" });
        forkSlots.push({ label: "O" + id, task: "load" });
      }
    }

    return {
      t: w.t,
      dock: { slots: dockSlots, queue: yard.length, cap: cap("dock_doors") },
      fork: { slots: forkSlots, queue: 0, cap: cap("forklifts") },
      pick: { slots: pickSlots, queue: orderQueue.length, cap: cap("pickers") },
      pack: { slots: packSlots, queue: 0, cap: cap("packing") },
      inv: { level: w.inv.level, waiting: w.inv.waiting },
      yard,
      orders: orderQueue,
      parked,
      kpi: {
        trucks: w.kpi.trucks,
        trucksTotal: cur.totals.trucks,
        orders: w.kpi.orders,
        ordersTotal: cur.totals.orders,
        shipped: w.kpi.shipped,
        preempts: w.kpi.preempts,
        stockouts: w.kpi.stockouts,
      },
      subtitle: w.lastNotable,
    };
  }

  function cap(key) {
    return cur.meta.resources[key].capacity;
  }

  /* =================================================================== *
   * Human-readable event text (ticker + subtitle)
   * =================================================================== */

  function describe(e) {
    const id = e.actor ? e.actor.id : null;
    const i = e.info || {};
    const cls = i.expedite ? "EXPEDITE" : "standard";
    switch (e.ev) {
      case "truck_arrive": return `Truck ${id} arrives at the yard`;
      case "truck_unload_start": return `Truck ${id} unloading`;
      case "putaway_acquire": return `Truck ${id} putaway started`;
      case "putaway_resume": return `Truck ${id} putaway resumes (${fmt1(i.remaining)} left)`;
      case "putaway_preempted": return `Truck ${id} putaway PREEMPTED — pallet parked (${fmt1(i.remaining)} left)`;
      case "putaway_done": return `Truck ${id} putaway done  +${fmt0(i.cases)} cases  (stock ${fmt0(i.level)})`;
      case "order_arrive": return `Order ${id} received  ${fmt0(i.cases)} cases  [${cls}]`;
      case "stockout_begin": return `Order ${id} hits a STOCKOUT — picker stalled`;
      case "stockout_end": return `Order ${id} picked through stockout (stalled ${fmt1(i.stalled)})`;
      case "load_start": return `Order ${id} loading onto trailer`;
      case "order_ship": return `Order ${id} shipped  [${cls}]`;
      case "gate_closed": return `Inbound gate closed (${i.trucks} trucks received)`;
      case "book_closed": return `Order book closed (${i.orders} orders accepted)`;
      case "day_cleared": return `Floor cleared`;
      default: return null; // pick_start/pick_end/pack_*/unload_end/qc_end: not ticker-worthy
    }
  }

  function pushTicker(e) {
    const s = describe(e);
    if (!s) return;
    tickerLines.push({ t: e.t, s });
    if (tickerLines.length > 40) tickerLines.shift();
  }

  function renderTicker() {
    const tail = tickerLines.slice(-1);
    dom.ticker.innerHTML = "";
    for (let i = 0; i < tail.length; i++) {
      const row = document.createElement("div");
      row.className = "row cur";
      row.textContent = `[t=${tail[i].t.toFixed(1)}]  ${tail[i].s}`;
      dom.ticker.appendChild(row);
    }
  }

  /* =================================================================== *
   * Transport / clock
   * =================================================================== */

  function selectRun(label) {
    const run = runs.get(label);
    if (!run) return;
    cur = run;
    world = null; // force a clean rebuild for the new run
    cursor = 0;
    tEnd = run.events.length ? run.events[run.events.length - 1].t : 1;
    scene.build(run.meta);
    tickerLines.length = 0;
    seekTo(0, false);
    dom.dropzone.classList.add("hidden");
    dom.seedSelect.value = label;
    dom.scrubber.max = String(Math.max(1, Math.ceil(tEnd)));
    if (pendingAutoplay) {
      pendingAutoplay = false;
      setPlaying(true);
    }
  }

  function seekTo(t, live) {
    if (!cur) return;
    simClock = clamp(t, 0, tEnd);
    const idx = eventsUpTo(cur.events, simClock);
    if (!world) rebuildTo(0); // ensure an initialized world before folding
    advanceTo(idx, live);
    paint();
  }

  function paint() {
    scene.update(renderState());
    dom.scrubber.value = String(simClock);
    dom.clock.textContent = "t " + simClock.toFixed(1);
    renderTicker();
  }

  function eventsUpTo(events, t) {
    let lo = 0,
      hi = events.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (events[mid].t <= t + 1e-9) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  function frame(now) {
    raf = requestAnimationFrame(frame);
    if (!cur) return;
    const dt = lastNow ? (now - lastNow) / 1000 : 0;
    lastNow = now;
    if (playing && !scrubbing) {
      const speed = parseFloat(dom.speed.value);
      let next = simClock + dt * BASE_RATE * speed;
      if (next >= tEnd) {
        next = tEnd;
        seekTo(next, true);
        if (dom.loop.checked) {
          // brief hold at end, then restart
          seekTo(0, false);
        } else {
          setPlaying(false);
        }
        return;
      }
      seekTo(next, true);
    }
  }

  function setPlaying(on) {
    playing = on;
    dom.play.textContent = on ? "⏸" : "▶";
    dom.play.title = on ? "Pause (Space)" : "Play (Space)";
    if (on && simClock >= tEnd) seekTo(0, false);
  }

  function stepEvent(dir) {
    setPlaying(false);
    if (!cur) return;
    let idx = cursor + dir;
    idx = clamp(idx, 0, cur.events.length);
    if (idx === 0) {
      seekTo(0, false);
      return;
    }
    // Seek to the time of the target event (inclusive of it).
    const targetT = cur.events[idx - 1].t;
    seekTo(targetT, false);
    // Ensure exactly idx events are applied (handles same-timestamp events).
    advanceTo(idx, false);
    paint();
  }

  /* =================================================================== *
   * UI wiring
   * =================================================================== */

  function wire() {
    // Drag & drop (whole window)
    const dz = dom.dropzone;
    window.addEventListener("dragover", (e) => {
      e.preventDefault();
      dz.classList.add("dragover");
    });
    window.addEventListener("dragleave", (e) => {
      if (e.relatedTarget === null) dz.classList.remove("dragover");
    });
    window.addEventListener("drop", (e) => {
      e.preventDefault();
      dz.classList.remove("dragover");
      if (e.dataTransfer && e.dataTransfer.files.length) loadFiles(e.dataTransfer.files);
    });

    dom.pickBtn.addEventListener("click", () => dom.fileInput.click());
    dom.loadMore.addEventListener("click", () => dom.fileInput.click());
    dom.fileInput.addEventListener("change", (e) => {
      if (e.target.files.length) loadFiles(e.target.files);
      e.target.value = "";
    });

    dom.play.addEventListener("click", () => setPlaying(!playing));
    dom.stepBack.addEventListener("click", () => stepEvent(-1));
    dom.stepFwd.addEventListener("click", () => stepEvent(+1));

    dom.scrubber.addEventListener("input", () => {
      scrubbing = true;
      seekTo(parseFloat(dom.scrubber.value), false);
    });
    dom.scrubber.addEventListener("change", () => {
      scrubbing = false;
    });

    dom.seedSelect.addEventListener("change", () => {
      const wasPlaying = playing;
      selectRun(dom.seedSelect.value);
      if (wasPlaying) setPlaying(true);
    });

    document.addEventListener("keydown", (e) => {
      if (e.target.tagName === "SELECT" || e.target.tagName === "INPUT") return;
      if (e.code === "Space") {
        e.preventDefault();
        setPlaying(!playing);
      } else if (e.code === "ArrowRight") {
        e.preventDefault();
        stepEvent(+1);
      } else if (e.code === "ArrowLeft") {
        e.preventDefault();
        stepEvent(-1);
      }
    });

    // URL flags: ?chrome=off  ?autoplay=1  ?loop=1  ?speed=2
    const q = new URLSearchParams(location.search);
    if (q.get("chrome") === "off") document.body.classList.add("no-chrome");
    if (q.get("loop") === "1") dom.loop.checked = true;
    if (q.get("speed")) dom.speed.value = q.get("speed");
    if (q.get("autoplay") === "1") {
      // Will start once the first file is dropped/loaded (see selectRun).
      pendingAutoplay = true;
    }
  }

  let pendingAutoplay = false;

  // Kick the rAF loop immediately; it idles until a run is loaded.
  function boot() {
    wire();
    raf = requestAnimationFrame(frame);
  }

  function clamp(x, lo, hi) {
    return x < lo ? lo : x > hi ? hi : x;
  }
  function fmt0(x) {
    return x == null ? "?" : String(Math.round(x));
  }
  function fmt1(x) {
    return x == null ? "?" : Number(x).toFixed(1);
  }

  boot();
})();

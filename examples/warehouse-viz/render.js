// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

/* ===================================================================== *
 * render.js — builds the static SVG warehouse floor plan from a `meta`
 * line and updates it from a per-frame `state` object computed by app.js.
 *
 * No frameworks, no modules (so it works from file://). Exposes a global
 * `Scene` class. The renderer is intentionally "dumb": it only paints the
 * state it is handed; all simulation/replay logic lives in app.js.
 * ===================================================================== */

const SVGNS = "http://www.w3.org/2000/svg";

/** Tiny SVG element helper. */
function el(tag, attrs, children) {
  const n = document.createElementNS(SVGNS, tag);
  if (attrs) for (const k in attrs) n.setAttribute(k, attrs[k]);
  if (children) for (const c of children) n.appendChild(c);
  return n;
}
function clear(node) {
  while (node.firstChild) node.removeChild(node.firstChild);
}

const reducedMotion =
  window.matchMedia &&
  window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/* --- Layout constants (viewBox 1280 x 720) --------------------------- */
const VB_W = 1280,
  VB_H = 720;

class Scene {
  constructor(svg) {
    this.svg = svg;
    this.nodes = {};
    this._stackSig = {};
  }

  /* ----- Build the static scene from a meta line -------------------- */
  build(meta) {
    clear(this.svg);
    this.nodes = {};
    this._stackSig = {};
    this.meta = meta;
    const caps = meta.resources;

    // Defs: arrowheads
    const defs = el("defs");
    const marker = el("marker", {
      id: "arrow",
      viewBox: "0 0 10 10",
      refX: "8",
      refY: "5",
      markerWidth: "7",
      markerHeight: "7",
      orient: "auto-start-reverse",
    });
    marker.appendChild(el("path", { d: "M0,0 L10,5 L0,10 z", fill: "#2c3a47" }));
    defs.appendChild(marker);
    this.svg.appendChild(defs);

    // Floor background
    this.svg.appendChild(el("rect", { x: 0, y: 0, width: VB_W, height: VB_H, fill: "var(--floor)" }));

    // ----- Zone bands -----
    const zones = [
      ["YARD", 18, 184],
      ["RECEIVING", 196, 332],
      ["STORAGE", 540, 232],
      ["SHIPPING", 786, 476],
    ];
    for (const [label, x, w] of zones) {
      this.svg.appendChild(el("rect", { x, y: 70, width: w, height: 566, rx: 10, class: "zone-band" }));
      this.svg.appendChild(
        el("text", { x: x + w / 2, y: 92, "text-anchor": "middle", class: "zone-label" }, [
          txt(label),
        ])
      );
    }

    // ----- KPI bar (top) -----
    this._buildKpiBar();

    // ----- Stations -----
    // Dock doors — receiving top (shared inbound/outbound)
    this.nodes.dock = this._buildStation({
      key: "dock",
      title: "Dock doors",
      cap: caps.dock_doors.capacity,
      x: 208, y: 116, w: 308, h: 150,
      slotKind: "dock",
    });
    // Forklifts — receiving bottom (the centerpiece)
    this.nodes.fork = this._buildStation({
      key: "fork",
      title: "Forklifts",
      cap: caps.forklifts.capacity,
      x: 208, y: 326, w: 308, h: 250,
      slotKind: "fork",
      glyph: "\uD83D\uDE9C", // tractor/forklift stand-in
      big: true,
    });
    // Inventory tank — storage
    this._buildTank({
      x: 566, y: 116, w: 180, h: 380,
      capacity: caps.inventory.capacity,
      initial: caps.inventory.initial,
    });
    // Staging lane (parked pallets) — storage bottom
    this.svg.appendChild(
      el("text", { x: 656, y: 540, "text-anchor": "middle", class: "station-lab" }, [txt("Staging")])
    );
    this.nodes.parked = el("g", { transform: "translate(556,552)" });
    this.svg.appendChild(this.nodes.parked);

    // Pickers — shipping top
    this.nodes.pick = this._buildStation({
      key: "pick",
      title: "Pickers",
      cap: caps.pickers.capacity,
      x: 800, y: 116, w: 308, h: 150,
      slotKind: "pick",
    });
    // Packing — shipping mid
    this.nodes.pack = this._buildStation({
      key: "pack",
      title: "Packing",
      cap: caps.packing.capacity,
      x: 800, y: 326, w: 200, h: 130,
      slotKind: "pack",
    });

    // Order queue — far right
    this.svg.appendChild(
      el("text", { x: 1192, y: 110, "text-anchor": "middle", class: "station-lab" }, [txt("Order queue")])
    );
    this.nodes.orders = el("g", { transform: "translate(1126,124)" });
    this.svg.appendChild(this.nodes.orders);

    // Yard truck queue
    this.svg.appendChild(
      el("text", { x: 110, y: 110, "text-anchor": "middle", class: "station-lab" }, [txt("Arriving")])
    );
    this.nodes.yard = el("g", { transform: "translate(34,124)" });
    this.svg.appendChild(this.nodes.yard);

    // ----- Flow arrows (left -> right story) -----
    this._arrow(110, 150, 110, 230, "");                  // yard -> docks (down into receiving)
    this._flowLine(516, 191, 566, 230, "unload");          // dock -> tank
    this._flowLine(516, 430, 560, 360, "putaway");         // fork -> tank
    this._flowLine(746, 300, 800, 191, "pick");            // tank -> pickers
    this._flowLine(900, 266, 900, 326, "");                // pickers -> packing
    this._flowLine(1000, 391, 1060, 470, "load");          // packing -> dock (load)
    // Shared-forklift annotation
    this.svg.appendChild(
      el("text", { x: 362, y: 600, "text-anchor": "middle", class: "flow-label" }, [
        txt("forklifts shared: unload / load (blue) vs putaway (amber)"),
      ])
    );

    // ----- In-frame subtitle (stays visible when chrome is hidden) ---
    this.nodes.subtitle = el("text", {
      x: VB_W / 2, y: 678, "text-anchor": "middle", class: "subtitle-text",
    });
    this.svg.appendChild(
      el("rect", { x: 40, y: 656, width: VB_W - 80, height: 34, rx: 7, fill: "rgba(12,17,22,0.7)", stroke: "var(--panel-edge)" })
    );
    this.svg.appendChild(this.nodes.subtitle);
  }

  /* ----- KPI bar ---------------------------------------------------- */
  _buildKpiBar() {
    this.svg.appendChild(el("rect", { x: 18, y: 8, width: VB_W - 36, height: 50, rx: 8, class: "kpi-box" }));
    const fields = [
      ["TIME", "t"],
      ["TRUCKS", "trucks"],
      ["ORDERS", "orders"],
      ["SHIPPED", "shipped"],
      ["PREEMPTS", "preempts"],
      ["STOCKOUTS", "stockouts"],
      ["FORKLIFTS", "fork"],
      ["INVENTORY", "inv"],
    ];
    this.nodes.kpi = {};
    const n = fields.length;
    const x0 = 40,
      span = (VB_W - 80) / n;
    fields.forEach(([label, key], i) => {
      const cx = x0 + span * i;
      this.svg.appendChild(el("text", { x: cx, y: 26, class: "kpi-key" }, [txt(label)]));
      const v = el("text", { x: cx, y: 47, class: "kpi-val" }, [txt("–")]);
      this.svg.appendChild(v);
      this.nodes.kpi[key] = v;
    });
  }

  /* ----- A capacity-many-slot station ------------------------------- */
  _buildStation(o) {
    const g = el("g");
    this.svg.appendChild(g);
    g.appendChild(el("rect", { x: o.x, y: o.y, width: o.w, height: o.h, rx: 8, class: "station-box" }));
    g.appendChild(el("text", { x: o.x + 12, y: o.y - 8, class: "station-lab" }, [txt(o.title)]));
    const capText = el("text", { x: o.x + o.w - 8, y: o.y - 8, "text-anchor": "end", class: "station-cap" }, [
      txt(`0/${o.cap}`),
    ]);
    g.appendChild(capText);

    const slots = [];
    const pad = 14;
    const innerY = o.y + 36;
    const innerH = o.h - 52;
    const cols = o.cap <= 4 ? o.cap : Math.ceil(o.cap / 2);
    const rows = Math.ceil(o.cap / cols);
    const sw = (o.w - pad * (cols + 1)) / cols;
    const sh = (innerH - pad * (rows - 1)) / rows;
    for (let i = 0; i < o.cap; i++) {
      const r = Math.floor(i / cols),
        c = i % cols;
      const sx = o.x + pad + c * (sw + pad);
      const sy = innerY + r * (sh + pad);
      const rect = el("rect", { x: sx, y: sy, width: sw, height: sh, rx: 6, class: "slot" });
      g.appendChild(rect);
      let glyph = null;
      if (o.glyph) {
        glyph = el("text", {
          x: sx + sw / 2, y: sy + sh / 2 - 6, "text-anchor": "middle",
          "dominant-baseline": "middle", class: "fork-glyph",
        }, [txt(o.glyph)]);
        g.appendChild(glyph);
      }
      const lab = el("text", {
        x: sx + sw / 2, y: sy + sh / 2 + (o.big ? 22 : 4),
        "text-anchor": "middle", "dominant-baseline": "middle", class: "token-label",
      });
      g.appendChild(lab);
      slots.push({ rect, lab, glyph, cx: sx + sw / 2, cy: sy + sh / 2 });
    }
    return { slots, capText, cap: o.cap, key: o.key };
  }

  /* ----- Inventory tank --------------------------------------------- */
  _buildTank(o) {
    this.svg.appendChild(el("text", { x: o.x + o.w / 2, y: o.y - 8, "text-anchor": "middle", class: "station-lab" }, [
      txt("Inventory"),
    ]));
    this.svg.appendChild(el("rect", { x: o.x, y: o.y, width: o.w, height: o.h, rx: 8, class: "tank-frame" }));
    const fill = el("rect", { x: o.x + 4, y: o.y + 4, width: o.w - 8, height: 0, rx: 4, class: "tank-fill" });
    this.svg.appendChild(fill);
    const label = el("text", { x: o.x + o.w / 2, y: o.y + o.h / 2, "text-anchor": "middle", class: "tank-label" });
    const capLab = el("text", { x: o.x + o.w / 2, y: o.y + o.h + 18, "text-anchor": "middle", class: "tank-cap" }, [
      txt(`capacity ${fmt(o.capacity)}`),
    ]);
    this.svg.appendChild(label);
    this.svg.appendChild(capLab);
    this.nodes.tank = { fill, label, x: o.x, y: o.y, w: o.w, h: o.h, capacity: o.capacity };
  }

  _arrow(x1, y1, x2, y2) {
    this.svg.appendChild(el("line", { x1, y1, x2, y2, class: "flow-arrow", "marker-end": "url(#arrow)" }));
  }
  _flowLine(x1, y1, x2, y2, label) {
    this.svg.appendChild(el("line", { x1, y1, x2, y2, class: "flow-arrow", "marker-end": "url(#arrow)" }));
    if (label) {
      this.svg.appendChild(
        el("text", { x: (x1 + x2) / 2, y: (y1 + y2) / 2 - 4, "text-anchor": "middle", class: "flow-label" }, [
          txt(label),
        ])
      );
    }
  }

  /* ----- Per-frame update ------------------------------------------- */
  update(s) {
    this._updateStation(this.nodes.dock, s.dock);
    this._updateStation(this.nodes.fork, s.fork);
    this._updateStation(this.nodes.pick, s.pick);
    this._updateStation(this.nodes.pack, s.pack);
    this._updateTank(s.inv);
    this._updateStack("yard", this.nodes.yard, s.yard.map((id) => ({ label: "T" + id, cls: "token-truck" })), s.dock.queue);
    this._updateStack("orders", this.nodes.orders,
      s.orders.map((o) => ({ label: "O" + o.id, cls: "token-order" + (o.exp ? " exp" : ""), sub: o.cases != null ? fmt(o.cases) : null })),
      s.pick.queue);
    this._updateStack("parked", this.nodes.parked,
      s.parked.map((p) => ({ label: "T" + p.id, cls: "parked-pallet", sub: p.remaining != null ? p.remaining.toFixed(0) : null })), 0, true);
    this._updateKpi(s);
    if (s.subtitle != null) clearText(this.nodes.subtitle, s.subtitle);
  }

  _updateStation(st, data) {
    st.capText.textContent = `${data.slots.length}/${st.cap}`;
    for (let i = 0; i < st.cap; i++) {
      const slot = st.slots[i];
      const occ = data.slots[i];
      let cls = "slot";
      let label = "";
      if (occ) {
        if (occ.task === "putaway") cls += " busy-putaway";
        else if (occ.task === "unload" || occ.task === "load") cls += " busy-truck";
        else if (st.key === "pick") cls += occ.exp ? " busy-pick-exp" : " busy-pick";
        else if (st.key === "pack") cls += " busy-pack";
        else cls += " busy-generic";
        if (occ.stalled) cls += " flash";
        label = occ.label || "";
      }
      slot.rect.setAttribute("class", cls);
      slot.lab.textContent = label;
      if (slot.glyph) slot.glyph.style.opacity = occ ? "1" : "0.25";
    }
  }

  _updateTank(inv) {
    const t = this.nodes.tank;
    const frac = Math.max(0, Math.min(1, inv.level / t.capacity));
    const fh = (t.h - 8) * frac;
    t.fill.setAttribute("y", t.y + 4 + (t.h - 8 - fh));
    t.fill.setAttribute("height", fh);
    const low = inv.level <= 0.5 || inv.waiting > 0;
    t.fill.setAttribute("class", "tank-fill" + (low ? " low" : ""));
    clearText(t.label, fmt(inv.level));
    t.label.setAttribute("y", t.y + t.h / 2);
  }

  _updateKpi(s) {
    const k = this.nodes.kpi;
    clearText(k.t, "t " + s.t.toFixed(1));
    clearText(k.trucks, `${s.kpi.trucks}/${s.kpi.trucksTotal}`);
    clearText(k.orders, `${s.kpi.orders}/${s.kpi.ordersTotal}`);
    clearText(k.shipped, "" + s.kpi.shipped);
    clearText(k.preempts, "" + s.kpi.preempts);
    setHot(k.preempts, s.kpi.preempts > 0);
    clearText(k.stockouts, "" + s.kpi.stockouts);
    setHot(k.stockouts, s.inv.waiting > 0);
    clearText(k.fork, `${s.fork.slots.length}/${s.fork.cap}`);
    clearText(k.inv, fmt(s.inv.level));
    setHot(k.inv, s.inv.waiting > 0);
  }

  /* ----- A variable-length token stack (yard / orders / parked) ----- */
  _updateStack(key, g, items, queueCount, horizontal) {
    const sig = items.map((i) => i.label + (i.sub || "")).join(",") + "|" + queueCount;
    if (this._stackSig[key] === sig) return;
    this._stackSig[key] = sig;
    clear(g);
    const MAX = horizontal ? 6 : 9;
    const shown = items.slice(0, MAX);
    shown.forEach((it, i) => {
      let tx, ty;
      if (horizontal) {
        tx = i * 78;
        ty = 0;
      } else {
        tx = 0;
        ty = i * 44;
      }
      const w = 64,
        h = 36;
      const tok = el("g", { transform: `translate(${tx},${ty})` });
      tok.appendChild(el("rect", { x: 0, y: 0, width: w, height: h, rx: 6, class: "token " + it.cls }));
      tok.appendChild(el("text", { x: w / 2, y: it.sub ? 16 : 22, class: "token-label" }, [txt(it.label)]));
      if (it.sub) tok.appendChild(el("text", { x: w / 2, y: 28, class: "token-sub" }, [txt(it.sub + (key === "orders" ? " cs" : ""))]));
      g.appendChild(tok);
    });
    const extra = items.length - shown.length;
    if (extra > 0) {
      const ty = horizontal ? 0 : shown.length * 44;
      const tx = horizontal ? shown.length * 78 : 0;
      g.appendChild(
        el("text", { x: tx + 6, y: ty + 22, class: "stack-more" }, [
          txt(`+${extra}`),
        ])
      );
    }
  }

  /* ----- Preemption / stockout flash -------------------------------- */
  flash(target) {
    let cx, cy;
    if (target === "fork") {
      cx = 362;
      cy = 450;
    } else if (target === "inv") {
      cx = 656;
      cy = 300;
    } else return;
    const ring = el("circle", { cx, cy, r: 30, class: "flash-ring" });
    this.svg.appendChild(ring);
    if (reducedMotion) {
      setTimeout(() => ring.remove(), 220);
      return;
    }
    ring.animate(
      [
        { r: 24, opacity: 0.95, strokeWidth: 5 },
        { r: 70, opacity: 0, strokeWidth: 1 },
      ],
      { duration: 480, easing: "ease-out" }
    ).onfinish = () => ring.remove();
  }
}

/* --- helpers --- */
function txt(s) {
  return document.createTextNode(s);
}
function clearText(node, s) {
  node.textContent = s;
}
function setHot(node, on) {
  node.setAttribute("class", "kpi-val" + (on ? " hot" : ""));
}
/** Format a number: integers plain, else 1 decimal. */
function fmt(x) {
  if (x == null) return "–";
  return Math.abs(x - Math.round(x)) < 1e-9 ? String(Math.round(x)) : x.toFixed(1);
}

window.Scene = Scene;

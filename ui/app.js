// battery-log — the window over the log.
//
// Data comes from the Rust side in one call (`dashboard`), plus a light `live`
// poll for the header. Charts are hand-rolled SVG: the whole point of this app
// is a dense, precise picture, and a charting library would cost more than it
// gives for six chart shapes.

const TAURI = window.__TAURI__;
const invoke = TAURI ? TAURI.core.invoke : null;

const state = {
  days: 30,
  tab: "day",
  day: null,
  data: null,
  live: null,
};

// ── plumbing ────────────────────────────────────────────────────────────────

const $ = (id) => document.getElementById(id);
const NS = "http://www.w3.org/2000/svg";
const css = (n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim();

async function call(cmd, args) {
  if (invoke) return invoke(cmd, args);
  // Opened outside the app (a browser, a screenshot harness): fall back to a
  // dumped snapshot so the UI can be worked on without the shell.
  const demo = await fetch("demo.json").then((r) => r.json());
  if (cmd === "dashboard") return demo;
  if (cmd === "live") return demo.battery;
  if (cmd === "palette") return demo._palette || {};
  if (cmd === "day_procs") return demo.top_procs.slice(0, 8);
  return null;
}

function applyPalette(p) {
  for (const [k, v] of Object.entries(p || {})) {
    document.documentElement.style.setProperty(`--${k}`, `#${v}`);
  }
}

const DASH = "—";
// "0h28m" is four characters of noise around one fact, and "0h00m" is a claim
// of zero where there is usually no reading at all.
const fmtH = (h) => {
  if (h == null) return DASH;
  const mins = Math.round(h * 60);
  if (mins < 1) return DASH;
  return mins < 60 ? `${mins}m` : `${Math.floor(mins / 60)}h${String(mins % 60).padStart(2, "0")}m`;
};
const clock = (m) =>
  `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(Math.round(m % 60)).padStart(2, "0")}`;
const shortDate = (iso) =>
  new Date(iso + "T12:00").toLocaleDateString(undefined, { month: "short", day: "numeric" });
const longDate = (iso) =>
  new Date(iso + "T12:00").toLocaleDateString(undefined,
    { weekday: "long", month: "long", day: "numeric" });
const pct = (v) => `${Math.round(v * 100)}%`;
// A missing number is not zero. Everywhere a value can be absent, say so in
// words rather than printing a confident 0.00.
const NONE = '<span class="none">not measurable</span>';
const watts = (w) => (w == null ? NONE : `${w.toFixed(2)} W`);
// Process CPU is often seconds, not minutes; "0 min" tells nobody anything.
const cpuTime = (s) => (s >= 60 ? `${(s / 60).toFixed(s < 600 ? 1 : 0)} min` : `${Math.round(s)} s`);

// ── chart engine ────────────────────────────────────────────────────────────

const tip = $("tip");

function el(tag, attrs = {}, parent) {
  const n = document.createElementNS(NS, tag);
  for (const k in attrs) if (attrs[k] != null) n.setAttribute(k, attrs[k]);
  if (parent) parent.appendChild(n);
  return n;
}

function ticks(lo, hi, count) {
  const span = hi - lo || 1;
  const raw = span / count;
  const mag = Math.pow(10, Math.floor(Math.log10(raw)));
  const step = [1, 2, 2.5, 5, 10].map((s) => s * mag).find((s) => s >= raw) || 10 * mag;
  const out = [];
  for (let v = Math.ceil(lo / step) * step; v <= hi + step / 1e6; v += step) out.push(+v.toFixed(6));
  return out;
}

function showTip(evt, html) {
  tip.innerHTML = html;
  tip.style.opacity = 1;
  const pad = 14, w = tip.offsetWidth, h = tip.offsetHeight;
  let x = evt.clientX + pad, y = evt.clientY - h - pad;
  if (x + w > innerWidth - 8) x = evt.clientX - w - pad;
  if (y < 8) y = evt.clientY + pad;
  tip.style.left = `${x}px`;
  tip.style.top = `${y}px`;
}
const hideTip = () => { tip.style.opacity = 0; };

// One routine for every chart. `spec.type` is "line" (default) or "bar".
function chart(host, spec) {
  if (!host) return;
  const render = () => {
    host.innerHTML = "";
    const W = host.clientWidth || 640;
    const H = spec.height || 180;
    const m = { t: 10, r: 12, b: 24, l: 46 };
    const iw = Math.max(10, W - m.l - m.r), ih = H - m.t - m.b;
    const values = spec.y.filter((v) => v != null);
    if (!values.length) {
      host.innerHTML = `<p class="empty">${spec.emptyText || "Nothing recorded in this range."}</p>`;
      return;
    }
    const svg = el("svg", { viewBox: `0 0 ${W} ${H}`, width: W, height: H, role: "img",
                            "aria-label": spec.label || "" }, host);
    const g = el("g", { transform: `translate(${m.l},${m.t})` }, svg);

    const yMax = spec.yMax != null ? spec.yMax : (Math.max(...values, spec.ref || 0) * 1.12 || 1);
    const yMin = spec.yMin != null ? spec.yMin : 0;
    const Y = (v) => ih - ((v - yMin) / (yMax - yMin || 1)) * ih;
    const bar = spec.type === "bar";
    const xMin = bar ? 0 : (spec.xMin != null ? spec.xMin : Math.min(...spec.x));
    const xMax = bar ? spec.x.length : (spec.xMax != null ? spec.xMax : Math.max(...spec.x));
    const X = (v) => ((v - xMin) / (xMax - xMin || 1)) * iw;

    for (const b of spec.bands || []) {
      const x0 = X(b.x0), x1 = X(b.x1);
      el("rect", { x: x0, y: 0, width: Math.max(1.5, x1 - x0), height: ih,
                   fill: b.color, opacity: b.opacity ?? 0.18 }, g);
    }
    for (const t of ticks(yMin, yMax, 4)) {
      el("line", { x1: 0, x2: iw, y1: Y(t), y2: Y(t), stroke: css("--surface1"),
                   "stroke-width": 1, opacity: t === 0 ? 0.9 : 0.5 }, g);
      const lab = el("text", { x: -8, y: Y(t) + 4, "text-anchor": "end",
                               fill: css("--overlay1"), "font-size": 10.5 }, g);
      lab.textContent = spec.yFmt ? spec.yFmt(t) : t;
    }
    if (spec.ref != null && spec.ref <= yMax) {
      el("line", { x1: 0, x2: iw, y1: Y(spec.ref), y2: Y(spec.ref), stroke: css("--overlay1"),
                   "stroke-width": 1, "stroke-dasharray": "4 5", opacity: .65 }, g);
      const t = el("text", { x: iw, y: Y(spec.ref) - 5, "text-anchor": "end",
                             fill: css("--overlay1"), "font-size": 10.5 }, g);
      t.textContent = spec.refLabel || "";
    }

    // Estimated bars are hatched rather than recoloured: colour already means
    // "which measure", so provenance gets the second channel.
    let hatch = null;
    if ((spec.hatched || []).some(Boolean)) {
      hatch = `hatch-${host.id}`;
      const pat = el("pattern", { id: hatch, width: 5, height: 5,
        patternUnits: "userSpaceOnUse", patternTransform: "rotate(45)" }, el("defs", {}, svg));
      el("rect", { width: 5, height: 5, fill: spec.color, opacity: .3 }, pat);
      el("rect", { width: 2, height: 5, fill: spec.color }, pat);
    }

    if (bar) {
      const slot = iw / spec.x.length;
      const bw = Math.max(2, Math.min(34, slot - 2));   // 2px surface gap
      spec.y.forEach((v, i) => {
        if (v == null) return;
        const h = Math.max(v > 0 ? 2 : 0, ih - Y(v));
        el("rect", { x: X(i) + (slot - bw) / 2, y: ih - h, width: bw, height: h,
                     rx: Math.min(4, bw / 2),
                     fill: spec.hatched?.[i] && hatch ? `url(#${hatch})`
                           : (spec.colors?.[i] || spec.color) }, g);
        const hit = el("rect", { x: X(i), y: 0, width: slot, height: ih, fill: "transparent" }, g);
        hit.addEventListener("pointermove", (e) => showTip(e, spec.tipFor(i)));
        hit.addEventListener("pointerleave", hideTip);
      });
      spec.x.forEach((lab, i) => {
        if (spec.x.length > 14 && i % Math.ceil(spec.x.length / 10)) return;
        const t = el("text", { x: X(i) + slot / 2, y: ih + 16, "text-anchor": "middle",
                               fill: css("--overlay1"), "font-size": 10.5 }, g);
        t.textContent = lab;
      });
      return;
    }

    (spec.xTicks || []).forEach((t, i, all) => {
      // The first and last labels sit against the plot edges; anchoring them
      // outward keeps them from being clipped by the viewBox.
      const anchor = i === 0 ? "start" : i === all.length - 1 ? "end" : "middle";
      const tx = el("text", { x: X(t.v), y: ih + 16, "text-anchor": anchor,
                              fill: css("--overlay1"), "font-size": 10.5 }, g);
      tx.textContent = t.label;
    });
    // A hole in the data is information -- the machine was asleep, or charging.
    // Break the line there instead of interpolating across it.
    const segs = [];
    let seg = [];
    spec.y.forEach((v, i) => {
      const gapped = i > 0 && spec.x[i] - spec.x[i - 1] > (spec.maxGap ?? Infinity);
      if (v == null || gapped) { if (seg.length) segs.push(seg); seg = []; }
      if (v != null) seg.push([X(spec.x[i]), Y(v)]);
    });
    if (seg.length) segs.push(seg);
    const pts = (s) => s.map(([x, y]) => `${x.toFixed(1)} ${y.toFixed(1)}`).join("L");
    if (spec.fill) {
      el("path", {
        d: segs.filter((s) => s.length > 1).map((s) =>
          `M${s[0][0].toFixed(1)} ${ih}L${pts(s)}L${s[s.length - 1][0].toFixed(1)} ${ih}Z`).join(""),
        fill: spec.color, opacity: .13,
      }, g);
    }
    el("path", { d: segs.filter((s) => s.length > 1).map((s) => `M${pts(s)}`).join(""),
                 fill: "none", stroke: spec.color, "stroke-width": 2,
                 "stroke-linejoin": "round", "stroke-linecap": "round" }, g);
    for (const s of segs) if (s.length === 1) {
      el("circle", { cx: s[0][0], cy: s[0][1], r: 3.5, fill: spec.color }, g);
    }

    const cross = el("line", { y1: 0, y2: ih, stroke: css("--surface2"), "stroke-width": 1, opacity: 0 }, g);
    const dot = el("circle", { r: 4, fill: spec.color, stroke: css("--base"), "stroke-width": 2, opacity: 0 }, g);
    const hit = el("rect", { x: 0, y: 0, width: iw, height: ih, fill: "transparent" }, g);
    hit.addEventListener("pointermove", (e) => {
      const rect = svg.getBoundingClientRect();
      const xv = xMin + ((e.clientX - rect.left - m.l) / iw) * (xMax - xMin);
      let best = -1, bd = Infinity;
      spec.x.forEach((xx, i) => {
        const dd = Math.abs(xx - xv);
        if (dd < bd && spec.y[i] != null) { bd = dd; best = i; }
      });
      if (best < 0) return;
      cross.setAttribute("x1", X(spec.x[best]));
      cross.setAttribute("x2", X(spec.x[best]));
      cross.setAttribute("opacity", .8);
      dot.setAttribute("cx", X(spec.x[best]));
      dot.setAttribute("cy", Y(spec.y[best]));
      dot.setAttribute("opacity", 1);
      showTip(e, spec.tipFor(best));
    });
    hit.addEventListener("pointerleave", () => {
      cross.setAttribute("opacity", 0);
      dot.setAttribute("opacity", 0);
      hideTip();
    });
  };
  render();
  new ResizeObserver(() => requestAnimationFrame(render)).observe(host);
}

/// Horizontal bars: the right shape for a handful of named things.
function barList(host, items, { color, max, fmt, sub } = {}) {
  if (!host) return;
  if (!items.length) {
    host.innerHTML = `<p class="empty">Nothing recorded yet.</p>`;
    return;
  }
  const top = max ?? (Math.max(...items.map((i) => i.value || 0)) || 1);
  host.innerHTML = items.map((i) => `
    <div class="bar">
      <span class="lab" title="${i.label}">${i.label}</span>
      <span class="track"><i class="fill" style="width:${Math.max(2, (i.value / top) * 100)}%;
        background:${i.color || color || "var(--draw)"}"></i></span>
      <span class="val">${fmt ? fmt(i) : i.value}${
        sub && sub(i) ? `<span class="sub">${sub(i)}</span>` : ""}</span>
    </div>`).join("");
}

// ── panels ──────────────────────────────────────────────────────────────────

const HOURS = [0, 3, 6, 9, 12, 15, 18, 21, 24].map((h) => ({ v: h * 60, label: `${String(h).padStart(2, "0")}:00` }));

function renderLive(b) {
  if (!b) return;
  const low = b.status === "Discharging" && (b.capacity ?? 100) <= 20;
  const color = b.status !== "Discharging" ? "var(--charge)" : low ? "var(--red)" : "var(--level)";
  const bits = [];
  if (b.power_w != null) bits.push(`${b.power_w.toFixed(1)} W`);
  if (b.runtime_h != null) bits.push(`${fmtH(b.runtime_h)} left`);
  if (b.profile) bits.push(b.profile);
  const ctx = [];
  if (b.backlight != null) ctx.push(`backlight ${Math.round(b.backlight)}%`);
  if (b.displays_on != null) {
    ctx.push(b.displays_on === 0 ? "screen off"
      : b.external_displays ? `${b.external_displays} external` : "panel only");
  }
  if (b.cpu_busy != null) ctx.push(`cpu ${b.cpu_busy.toFixed(0)}%`);
  if (b.temp_c != null) ctx.push(`${Math.round(b.temp_c)}°C`);
  $("live").innerHTML = `
    <span class="cap">${b.capacity ?? "--"}%</span>
    <span class="gauge"><i style="width:${b.capacity ?? 0}%;background:${color}"></i></span>
    <span>${b.status.toLowerCase()}</span>
    <i class="sep"></i><span class="grp">${bits.map((x) => `<span>${x}</span>`).join("")}</span>
    <i class="sep"></i><span class="grp dim">${ctx.map((x) => `<span>${x}</span>`).join("")}</span>`;
}

function renderTiles(d) {
  const b = d.battery || {};
  const onBattery = d.days.filter((x) => x.battery_h > 0.05);
  const wh = onBattery.reduce((s, x) => s + x.drained_wh, 0);
  const hours = onBattery.reduce((s, x) => s + x.battery_h, 0);
  const avgW = hours > 0 ? wh / hours : null;
  const sleeps = d.sleeps.filter((s) => !s.powered_off && s.hours > 0.05);
  // Only sleeps with a readable drain make an average. A suspend spent on the
  // charger has no standby number, and averaging it in as zero would flatter.
  const known = sleeps.filter((s) => s.watts != null);
  const slept = known.reduce((s, x) => s + x.hours, 0);
  const standbyW = slept > 0 ? known.reduce((s, x) => s + x.drained_wh, 0) / slept : null;
  const resid = sleeps.filter((s) => s.hw_residency != null);
  // Weighted by how long each suspend lasted: a 16-hour sleep at 7% and a
  // 20-minute one at 90% do not average to half.
  const residH = resid.reduce((s, x) => s + x.hours, 0);
  const hwPct = residH ? resid.reduce((s, x) => s + x.hw_residency * x.hours, 0) / residH : null;
  const exact = sleeps.filter((s) => s.exact).length;
  const estDays = d.days.filter((x) => x.estimated).length;
  const withFull = d.days.filter((x) => x.full_wh != null);
  const full = withFull.length ? withFull[withFull.length - 1].full_wh : null;

  const tiles = [
    // The live `energy_full` swings several Wh through the day, so the health
    // headline reads the same median the chart plots rather than whichever
    // number the firmware happened to be showing this second.
    { k: "Health", v: full && b.design_wh ? `${(100 * full / b.design_wh).toFixed(1)}%` : DASH,
      d: full ? `${full.toFixed(1)} Wh of ${b.design_wh} design, ${b.cycles ?? 0} cycles` : "" },
    { k: "Average draw", v: avgW ? `${avgW.toFixed(1)} W` : DASH,
      d: `over ${fmtH(hours)} on battery`
         + (estDays ? `, ${estDays} of ${d.days.length} days estimated` : "") },
    { k: "Runtime at that rate", v: avgW && full ? fmtH(full / avgW) : DASH,
      d: "from a full charge" },
    { k: "Standby drain", v: standbyW != null ? `${standbyW.toFixed(2)} W` : DASH,
      d: standbyW == null ? "no measurable suspend yet"
        : (full ? `${((standbyW * 24) / full * 100).toFixed(0)}% of a charge per day, ` : "")
          + `from ${known.length} of ${sleeps.length} suspends` },
    { k: "Hardware sleep", v: hwPct != null ? pct(hwPct) : DASH,
      d: hwPct != null ? `of ${fmtH(residH)} the kernel could account for`
                       : "the kernel has not reported one yet",
      warn: hwPct != null && hwPct < 0.5 },
    { k: "On battery", v: fmtH(hours / Math.max(1, onBattery.length)),
      d: `per day, over ${onBattery.length} days` },
  ];
  $("tiles").innerHTML = tiles.map((t) => `
    <div class="tile${t.warn ? " warn" : ""}">
      <div class="k">${t.k}</div><div class="v">${t.v}</div><div class="d">${t.d || ""}</div>
    </div>`).join("");
}

function bandsFor(d, day) {
  const c = d.curve[day];
  const out = [];
  if (!c) return out;
  let start = null;
  c.state.forEach((s, i) => {
    if (s === 2 && start == null) start = c.t[i];
    if (s !== 2 && start != null) { out.push({ x0: start, x1: c.t[i], color: css("--charge") }); start = null; }
  });
  if (start != null) out.push({ x0: start, x1: c.t[c.t.length - 1], color: css("--charge") });
  // A suspend that ran from 22:00 to 14:00 belongs to the tail of one day and
  // the head of the next; clip it to whichever day is on screen.
  const dayStart = new Date(`${day}T00:00`).getTime() / 1000;
  for (const s of d.sleeps) {
    const from = Math.max(s.start, dayStart);
    const to = Math.min(s.end, dayStart + 86400);
    if (to <= from) continue;
    out.push({ x0: (from - dayStart) / 60, x1: (to - dayStart) / 60,
               color: css(s.powered_off ? "--surface2" : "--sleep"), opacity: .24 });
  }
  return out;
}

async function renderDay(d) {
  const days = Object.keys(d.curve).sort();
  if (!state.day || !days.includes(state.day)) state.day = days[days.length - 1];
  const at = days.indexOf(state.day);
  $("dayLabel").textContent = longDate(state.day);
  $("dayPrev").disabled = at <= 0;
  $("dayNext").disabled = at >= days.length - 1;

  const c = d.curve[state.day];
  const rec = d.days.find((x) => x.date === state.day) || {};
  const bands = bandsFor(d, state.day);
  const watts = (c?.w || []).filter((w) => w != null);

  $("dayNote").textContent = rec.date
    ? `${(rec.drained_wh ?? 0).toFixed(1)} Wh used over ${fmtH(rec.battery_h)} on battery`
      + (rec.charged_wh ? `, ${rec.charged_wh.toFixed(1)} Wh put back` : "")
      + (rec.asleep_h ? `, ${fmtH(rec.asleep_h)} asleep` : "")
      + (rec.unaccounted_h > 0.05 ? `, ${fmtH(rec.unaccounted_h)} unaccounted for` : "")
    : "";
  $("drawNote").textContent = watts.length
    ? `Peak ${Math.max(...watts).toFixed(1)} W, averaging ${rec.avg_w != null ? rec.avg_w.toFixed(1) : DASH} W.`
    : "No time on battery this day.";

  const tipAt = (i) => `<b>${clock(c.t[i])}</b>`
    + (c.cap[i] != null ? `<div class="row">${c.cap[i]}% charge</div>` : "")
    + (c.w[i] != null ? `<div class="row">${c.w[i].toFixed(1)} W</div>` : "")
    + (c.cpu[i] != null ? `<div class="row">cpu ${c.cpu[i].toFixed(0)}%</div>` : "")
    + `<div class="row">${["idle", "on battery", "charging", "on AC"][c.state[i]]}</div>`;

  const common = { xMin: 0, xMax: 1440, xTicks: HOURS, bands, maxGap: 8, tipFor: tipAt };
  chart($("chartLevel"), { ...common, x: c?.t || [], y: c?.cap || [], color: css("--level"),
    yMin: 0, yMax: 100, height: 190, yFmt: (v) => `${v}%`, label: `Charge on ${state.day}` });
  chart($("chartDraw"), { ...common, x: c?.t || [], y: c?.w || [], color: css("--draw"), fill: true,
    height: 150, yFmt: (v) => `${v} W`, emptyText: "No time on battery this day.",
    label: `Draw on ${state.day}` });
  chart($("chartCpu"), { ...common, x: c?.t || [], y: c?.cpu || [], color: css("--cpu"), fill: true,
    yMin: 0, yMax: 100, height: 130, yFmt: (v) => `${v}%`,
    emptyText: "CPU context starts with the new daemon — check back tomorrow.",
    label: `CPU on ${state.day}` });

  const procs = await call("day_procs", { date: state.day, limit: 8 });
  const total = procs.reduce((s, p) => s + p.cpu_s, 0);
  $("dayProcNote").textContent = procs.length
    ? `${(total / 60).toFixed(0)} CPU-minutes while on battery`
    : "nothing recorded on battery this day";
  barList($("dayProcs"), procs.map((p) => ({ label: p.comm, value: p.cpu_s })), {
    color: "var(--cpu)", fmt: (i) => cpuTime(i.value),
  });
}

function renderHistory(d) {
  const labels = d.days.map((x) => shortDate(x.date));
  chart($("chartDaily"), {
    type: "bar", x: labels, y: d.days.map((x) => x.drained_wh), color: css("--draw"),
    hatched: d.days.map((x) => x.estimated),
    yFmt: (v) => `${v} Wh`, label: "Energy used per day",
    tipFor: (i) => `<b>${d.days[i].date}</b>
      <div class="row">${d.days[i].drained_wh.toFixed(1)} Wh used</div>
      <div class="row">${fmtH(d.days[i].battery_h)} on battery</div>
      ${d.days[i].avg_w ? `<div class="row">${d.days[i].avg_w} W average</div>` : ""}
      ${d.days[i].unaccounted_h > 0.5 ? `<div class="row">${fmtH(d.days[i].unaccounted_h)} unaccounted</div>` : ""}
      ${d.days[i].estimated ? `<div class="row">estimated from upower</div>` : ""}`,
  });
  chart($("chartDailyW"), {
    type: "bar", x: labels, y: d.days.map((x) => x.avg_w), color: css("--level"),
    hatched: d.days.map((x) => x.estimated),
    yFmt: (v) => `${v} W`, label: "Average draw per day",
    tipFor: (i) => `<b>${d.days[i].date}</b>
      <div class="row">${d.days[i].avg_w} W average</div>
      <div class="row">${fmtH(d.days[i].battery_h)} on battery</div>
      ${d.days[i].backlight != null ? `<div class="row">backlight ${Math.round(d.days[i].backlight)}%</div>` : ""}
      ${d.days[i].cpu_busy != null ? `<div class="row">cpu ${d.days[i].cpu_busy.toFixed(0)}%</div>` : ""}
      ${d.days[i].estimated ? `<div class="row">estimated from upower</div>` : ""}`,
  });

  const health = d.days.filter((x) => x.full_wh != null);
  const design = d.battery?.design_wh;
  const latest = health.length ? health[health.length - 1].full_wh : null;
  // Scaled to the readings, not to the design rating: against 83 Wh a 2 Wh
  // drift is a flat line, and a flat line is not what this is measuring.
  const lo = health.length ? Math.min(...health.map((x) => x.full_wh)) : 0;
  const hi = health.length ? Math.max(...health.map((x) => x.full_wh)) : 1;
  $("healthNote").textContent =
    (latest && design ? `${latest} Wh against a ${design} Wh design rating. ` : "")
    + `The firmware re-estimates capacity all day, so each point is that day's median reading.`
    + (health.length < 5 ? ` ${health.length} days in — too few to read a trend from.` : "");
  chart($("chartHealth"), {
    x: health.map((_, i) => i), y: health.map((x) => x.full_wh), color: css("--sapphire"),
    fill: health.length >= 5, height: 160,
    yMin: lo - Math.max(0.5, (hi - lo) * 0.4), yMax: hi + Math.max(0.5, (hi - lo) * 0.4),
    yFmt: (v) => `${v.toFixed(0)} Wh`,
    emptyText: "No full-charge reading in this range yet.",
    xTicks: health.length ? [{ v: 0, label: shortDate(health[0].date) },
                             { v: health.length - 1, label: shortDate(health[health.length - 1].date) }] : [],
    tipFor: (i) => `<b>${health[i].date}</b>
      <div class="row">${health[i].full_wh.toFixed(2)} Wh full charge</div>
      ${design ? `<div class="row">${(100 * health[i].full_wh / design).toFixed(1)}% of design</div>` : ""}
      ${health[i].cycles ? `<div class="row">${health[i].cycles} cycles</div>` : ""}`,
  });

  $("table").innerHTML = `<table><thead><tr>
      <th>Day</th><th>On battery</th><th>Used</th><th>Avg</th><th>Charged</th>
      <th>Asleep</th><th>Standby</th><th>Unaccounted</th><th>Backlight</th><th>CPU</th><th>Range</th>
    </tr></thead><tbody>` +
    d.days.slice().reverse().map((x) => `<tr class="${x.estimated ? "est" : ""}">
      <td>${x.date}${x.estimated ? ' <span class="tag">est</span>' : ""}</td>
      <td>${fmtH(x.battery_h)}</td><td>${x.drained_wh.toFixed(1)} Wh</td>
      <td>${x.avg_w != null ? `${x.avg_w.toFixed(1)} W` : DASH}</td><td>${x.charged_wh.toFixed(1)} Wh</td>
      <td>${fmtH(x.asleep_h)}</td><td>${x.standby_w != null ? `${x.standby_w.toFixed(2)} W` : DASH}</td>
      <td>${fmtH(x.unaccounted_h)}</td>
      <td>${x.backlight != null ? `${Math.round(x.backlight)}%` : DASH}</td>
      <td>${x.cpu_busy != null ? `${x.cpu_busy.toFixed(0)}%` : DASH}</td>
      <td>${x.min_cap != null ? `${x.min_cap}–${x.max_cap}%` : DASH}</td>
    </tr>`).join("") + `</tbody></table>`;
}

function renderSleep(d) {
  const sleeps = d.sleeps.slice().reverse();
  const real = sleeps.filter((s) => !s.powered_off);
  const known = real.filter((s) => s.watts != null);
  const slept = real.reduce((s, x) => s + x.hours, 0);
  const lost = known.reduce((s, x) => s + x.drained_wh, 0);
  const offs = sleeps.length - real.length;
  $("sleepNote").textContent =
    `${real.length} suspends, ${fmtH(slept)} asleep. `
    + (known.length
        ? `${lost.toFixed(1)} Wh lost across the ${known.length} with a readable drain.`
        : "None of them has a readable drain yet.")
    + (offs ? ` ${offs} power-off${offs > 1 ? "s" : ""}.` : "")
    + " A filled mark means the sampler was awake for both ends of the suspend;"
    + " hardware-sleep residency is only there for those.";

  $("sleepList").innerHTML = sleeps.slice(0, 60).map((s) => {
    const from = new Date(s.start * 1000), to = new Date(s.end * 1000);
    const r = s.hw_residency;
    const bad = s.watts != null && s.watts > 0.6;
    // Why a drain is missing is worth as much as the drain itself.
    const draw = s.powered_off ? '<span class="none">powered off</span>'
      : s.on_ac ? '<span class="none">on the charger</span>'
      : s.watts == null ? NONE
      : `${s.watts.toFixed(2)} W`;
    return `<div class="sleep-row${bad ? " bad" : ""}">
      <span class="when">
        <i class="mark${s.exact ? " exact" : ""}" title="${
          s.exact ? "sampled either side of the suspend" : "reconstructed from the hole in the log"}"></i>
        ${from.toLocaleString(undefined, { weekday: "short", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })}
        <span class="muted">→ ${to.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}</span></span>
      <span>${fmtH(s.hours)}</span>
      <span class="muted">${s.cap_from ?? "?"}% → ${s.cap_to ?? "?"}%</span>
      <span>${draw}</span>
      ${r != null
        ? `<span class="resid" title="share of the suspend the platform spent in real hardware sleep">
             <span class="track"><i class="fill" style="width:${Math.max(1.5, r * 100)}%"></i></span>
             <span class="muted">${pct(r)} hw</span></span>`
        : `<span></span>`}
    </div>`;
  }).join("") || `<p class="empty">No suspends logged yet.</p>`;

  const recent = known.slice(0, 40).reverse();
  chart($("chartSleep"), {
    type: "bar",
    x: recent.map((s) => new Date(s.start * 1000).toLocaleString(undefined,
      { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })),
    y: recent.map((s) => s.watts),
    hatched: recent.map((s) => !s.exact),
    color: css("--sleep"), yFmt: (v) => `${v} W`, label: "Standby drain per sleep",
    emptyText: "No suspend with a readable drain yet.",
    tipFor: (i) => {
      const s = recent[i];
      return `<b>${new Date(s.start * 1000).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })}</b>
        <div class="row">${fmtH(s.hours)} asleep</div>
        <div class="row">${s.drained_wh.toFixed(2)} Wh (${s.watts.toFixed(2)} W)</div>
        ${s.hw_residency != null ? `<div class="row">${pct(s.hw_residency)} in hardware sleep</div>` : ""}
        <div class="row">${s.cap_from}% → ${s.cap_to}%</div>
        <div class="row">${s.exact ? "measured at the boundary" : "inferred from the gap"}</div>`;
    },
  });
}

function bucketList(host, buckets, color) {
  const usable = buckets.filter((b) => b.avg_w != null);
  // These cards exist to compare bands against each other. With one band there
  // is nothing to compare, so say what is missing instead of drawing one bar.
  const card = $(host)?.closest(".card");
  if (card) card.hidden = usable.length < 2;
  barList($(host), usable.map((b) => ({ label: b.label, value: b.avg_w })), {
    color, fmt: (i) => `${i.value.toFixed(1)} W`,
    sub: (i) => {
      const b = buckets.find((x) => x.label === i.label);
      return b ? fmtH(b.hours) : "";
    },
  });
}

function renderUsage(d) {
  const total = d.top_procs.reduce((s, p) => s + p.cpu_s, 0);
  $("procNote").textContent = d.top_procs.length
    ? `${(total / 3600).toFixed(1)} CPU-hours while on battery, across ${d.range_days} days.`
      + ` Names come from the kernel and stop at 15 characters.`
    : "Nothing recorded yet — process attribution starts with the new sampler.";
  barList($("procs"), d.top_procs.map((p) => ({ label: p.comm, value: p.cpu_s })), {
    color: "var(--cpu)", fmt: (i) => cpuTime(i.value),
  });
  bucketList("byDisplay", d.by_display, "var(--level)");
  bucketList("byBacklight", d.by_backlight, "var(--yellow)");
  bucketList("byCpu", d.by_cpu, "var(--cpu)");
  bucketList("byProfile", d.by_profile, "var(--green)");
  const hourH = d.by_hour.reduce((s, b) => s + b.hours, 0);
  $("hourNote").textContent =
    `Average draw in each hour, over ${fmtH(hourH)} on battery. Unlike the bands above,`
    + ` this includes the estimated days — the seeded rows have no context, but they do have a clock.`;
  chart($("chartHour"), {
    type: "bar", x: d.by_hour.map((b) => b.label), y: d.by_hour.map((b) => b.avg_w),
    color: css("--draw"), yFmt: (v) => `${v} W`, label: "Average draw by hour",
    tipFor: (i) => `<b>${d.by_hour[i].label}:00</b>
      <div class="row">${d.by_hour[i].avg_w != null ? `${d.by_hour[i].avg_w} W average` : "never on battery"}</div>
      <div class="row">${fmtH(d.by_hour[i].hours)} sampled</div>`,
  });
}

// ── shell ───────────────────────────────────────────────────────────────────

async function render() {
  const d = state.data;
  if (!d) return;
  renderTiles(d);
  if (state.tab === "day") await renderDay(d);
  if (state.tab === "history") renderHistory(d);
  if (state.tab === "sleep") renderSleep(d);
  if (state.tab === "usage") renderUsage(d);
  const unacc = d.days.reduce((s, x) => s + x.unaccounted_h, 0);
  $("footer").innerHTML =
    `<span>${d.log.samples.toLocaleString()} samples since ${d.log.since ?? "?"}`
    + (d.log.seeded ? `, ${d.log.seeded.toLocaleString()} of them seeded from upower` : "")
    + `</span>`
    + (unacc > 0.5 ? `<span>${fmtH(unacc)} unaccounted for</span>` : "")
    + `<span class="grow"></span>`
    + `<span>${(d.log.bytes / 1e6).toFixed(1)} MB</span>`
    + `<span>updated ${new Date(d.generated * 1000).toLocaleTimeString()}</span>`;
}

async function reload() {
  state.data = await call("dashboard", { days: state.days, curveDays: 14, curve_days: 14 });
  await render();
}

/// The day view walks one day at a time. A row of thirty chips was a row of
/// thirty buttons for a control that is almost always used as "the day before".
function stepDay(delta) {
  if (state.tab !== "day" || !state.data) return;
  const days = Object.keys(state.data.curve).sort();
  const next = days[days.indexOf(state.day) + delta];
  if (!next) return;
  state.day = next;
  renderDay(state.data);
}

function showTab(tab) {
  state.tab = tab;
  history.replaceState(null, "", `#${tab}`);
  for (const b of document.querySelectorAll(".tab")) {
    b.setAttribute("aria-pressed", String(b.dataset.tab === tab));
  }
  for (const p of document.querySelectorAll(".panel")) {
    p.hidden = p.dataset.panel !== tab;
  }
  render();
}

async function boot() {
  applyPalette(await call("palette"));
  $("tabs").addEventListener("click", (e) => {
    const b = e.target.closest(".tab");
    if (b) showTab(b.dataset.tab);
  });
  $("dayPrev").addEventListener("click", () => stepDay(-1));
  $("dayNext").addEventListener("click", () => stepDay(1));
  $("range").addEventListener("change", (e) => {
    state.days = +e.target.value;
    reload();
  });
  if (TAURI) {
    const win = TAURI.window.getCurrentWindow();
    $("min").addEventListener("click", () => win.minimize());
    $("max").addEventListener("click", () => win.toggleMaximize());
    $("close").addEventListener("click", () => win.close());
  } else {
    document.querySelector(".winbtns").hidden = true;
  }
  addEventListener("keydown", (e) => {
    const tabs = ["day", "history", "sleep", "usage"];
    if (e.key >= "1" && e.key <= "4") showTab(tabs[+e.key - 1]);
    if (e.key === "ArrowLeft") stepDay(-1);
    if (e.key === "ArrowRight") stepDay(1);
    if (e.key === "r") reload();
  });

  const wanted = location.hash.replace("#", "");
  if (["day", "history", "sleep", "usage"].includes(wanted)) state.tab = wanted;
  showTab(state.tab);
  await reload();
  renderLive(state.data?.battery);
  setInterval(async () => renderLive(await call("live")), 5000);
  setInterval(reload, 60000);
}

boot();

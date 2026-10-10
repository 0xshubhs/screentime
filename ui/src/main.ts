import "./style.css";
import { barChart, byteScale, countScale, durationScale, durationTick, hideTip, hourScale, meter, type Column } from "./charts";
import * as dates from "./dates";
import { bytes, clock, count, dur, esc, pct } from "./format";
import type { AppCount, Filter, Report, Status, View } from "./types";

// Category colors follow the category, never its rank. The order is the validated
// categorical slot order, which is also the stacking order in bars.
const CATEGORY_SLOTS = [
  "Productivity & Finance",
  "Entertainment",
  "Social",
  "Information & Reading",
  "Creativity",
  "Education",
  "Developer Tools",
  "Games",
];
const OTHER = "Other";

function catColor(name: string): string {
  const i = CATEGORY_SLOTS.indexOf(name);
  return i >= 0 ? `var(--c${i + 1})` : "var(--c-other)";
}

/** Categories beyond the eight slots fold into "Other" in charts. */
function chartGroup(name: string): string {
  return CATEGORY_SLOTS.includes(name) ? name : OTHER;
}

interface State {
  view: View;
  anchor: string;
  tab: "apps" | "sites" | "categories";
  expanded: Record<string, boolean>;
}

const state: State = { view: "day", anchor: dates.today(), tab: "apps", expanded: {} };
let report: Report | null = null;
/** The week around the selected day, for the day view's week strip. */
let weekReport: Report | null = null;
let status: Status | null = null;
let loadSeq = 0;

// Routing: #/day/2026-10-10

function readHash() {
  const [, view, date] = location.hash.split("/");
  if (view === "day" || view === "week" || view === "month") state.view = view;
  if (date && /^\d{4}-\d{2}-\d{2}$/.test(date)) state.anchor = date;
}

function go(view: View, anchor: string) {
  location.hash = `#/${view}/${anchor}`;
}

window.addEventListener("hashchange", () => {
  readHash();
  load();
});

// Data

async function getJson<T>(url: string): Promise<T> {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${r.status} ${await r.text()}`);
  return r.json() as Promise<T>;
}

function reportUrl(from: string, to: string, filter?: Filter): string {
  const q = new URLSearchParams({ from, to });
  if (filter) q.set(filter.kind, filter.id);
  return `/api/report?${q}`;
}

async function load() {
  const seq = ++loadSeq;
  const { from, to } = dates.range(state.view, state.anchor);
  try {
    const week = dates.range("week", state.anchor);
    const [r, s, w] = await Promise.all([
      getJson<Report>(reportUrl(from, to)),
      getJson<Status>("/api/status"),
      state.view === "day" ? getJson<Report>(reportUrl(week.from, week.to)) : Promise.resolve(null),
    ]);
    if (seq !== loadSeq) return;
    report = r;
    status = s;
    weekReport = w;
    render();
  } catch (e) {
    if (seq !== loadSeq) return;
    app.innerHTML = `<div class="empty-page"><h1>Can't reach screentimed</h1><p>${esc(String(e))}</p>
      <p>Start it with <code>systemctl --user start screentimed</code>.</p></div>`;
  }
}

// Rendering helpers

const app = document.getElementById("app")!;

function icon(id: string, name: string, hasIcon: boolean, category?: string): string {
  if (hasIcon) return `<img class="icon" src="/api/icon?id=${encodeURIComponent(id)}" alt="" loading="lazy">`;
  const tint = category ? catColor(category) : "var(--ink-3)";
  return `<div class="icon letter" style="--tint:${tint}">${esc((name.trim()[0] ?? "?").toUpperCase())}</div>`;
}

function siteIcon(domain: string, category: string): string {
  return icon("", domain.replace(/^www\./, ""), false, category);
}

function isCurrent(): boolean {
  const { from, to } = dates.range(state.view, state.anchor);
  const t = dates.today();
  return from <= t && t <= to;
}

function compareText(r: Report): string {
  if (r.compare_avg_ms == null || r.compare_avg_ms === 0) return "";
  const p = pct(state.view === "day" ? r.total_ms : r.daily_avg_ms, r.compare_avg_ms);
  const against = state.view === "day" ? "your 7-day average" : state.view === "week" ? "last week" : "last month";
  if (p === 0) return `Same as ${against}`;
  const arrow = p > 0 ? "↑" : "↓";
  return `<span class="delta">${arrow} ${Math.abs(p)}%</span> ${p > 0 ? "above" : "below"} ${against}`;
}

function tickFor(i: number, label: string, r: Report): string {
  if (r.bucket === "hour") {
    const h = Number(label);
    return h % 6 === 0 ? (h === 0 ? "12 AM" : h === 12 ? "12 PM" : h < 12 ? `${h} AM` : `${h - 12} PM`) : "";
  }
  if (r.buckets.length <= 7) return label[0];
  const day = Number(label.split(" ")[1]);
  return day === 1 || day % 7 === 1 || i === r.buckets.length - 1 ? String(day) : "";
}

function bucketHeading(i: number, r: Report): string {
  const b = r.buckets[i];
  if (r.bucket === "hour") {
    const end = r.buckets[i + 1]?.start_ms ?? b.start_ms + 3_600_000;
    return `${clock(b.start_ms)} to ${clock(end)}`;
  }
  return new Date(b.start_ms).toLocaleDateString([], { weekday: "long", day: "numeric", month: "short" });
}

function dayOf(i: number, r: Report): string {
  return dates.ymd(new Date(r.buckets[i].start_ms));
}

function usageColumns(r: Report, colorFor?: string): Column[] {
  const t = dates.today();
  return r.buckets.map((b, i) => {
    let parts;
    if (colorFor) {
      parts = [{ name: "Usage", value: b.total_ms, color: colorFor }];
    } else {
      const grouped: Record<string, number> = {};
      for (const [cat, ms] of Object.entries(b.categories)) grouped[chartGroup(cat)] = (grouped[chartGroup(cat)] ?? 0) + ms;
      parts = [...CATEGORY_SLOTS, OTHER].map((name) => ({ name, value: grouped[name] ?? 0, color: catColor(name) }));
    }
    const day = r.bucket === "day" ? dayOf(i, r) : "";
    return {
      tick: tickFor(i, b.label, r),
      heading: bucketHeading(i, r),
      parts,
      highlight: r.bucket === "day" && day === t,
      onClick: r.bucket === "day" && day <= t ? () => go("day", day) : undefined,
    };
  });
}

function countColumns(r: Report, values: number[], color: string): Column[] {
  return values.map((v, i) => ({
    tick: tickFor(i, r.buckets[i].label, r),
    heading: bucketHeading(i, r),
    parts: [{ name: "Count", value: v, color }],
  }));
}

function listRows<T>(key: string, items: T[], row: (item: T) => string, limit = 6): string {
  if (items.length === 0) return `<p class="empty">Nothing recorded yet.</p>`;
  const open = state.expanded[key];
  const shown = open ? items : items.slice(0, limit);
  let h = `<ul class="rows">${shown.map(row).join("")}</ul>`;
  if (items.length > limit) {
    h += `<button class="more" data-expand="${key}">${open ? "Show less" : `Show all ${items.length}`}</button>`;
  }
  return h;
}

function countRow(a: AppCount, max: number, unit: string): string {
  return `<li class="row" data-app="${esc(a.id)}" data-name="${esc(a.name)}" data-icon="${a.has_icon ? 1 : 0}">
    ${icon(a.id, a.name, a.has_icon)}
    <div class="row-main"><div class="row-top"><span class="name">${esc(a.name)}</span><span class="value">${count(a.count)}${unit}</span></div>
    ${meter(a.count / max, "var(--accent)")}</div></li>`;
}

// Page

function render() {
  const r = report!;
  const s = status!;
  const multi = state.view !== "day";
  const current = isCurrent();
  const atToday = dates.range(state.view, state.anchor).to >= dates.today();

  const banners: string[] = [];
  if (s.tracking && !s.extension) {
    banners.push(`<div class="banner warn"><b>App tracking is waiting for the GNOME extension.</b>
      Enable it with <code>gnome-extensions enable screentime@0xshubhs</code> (log out and back in once after installing).</div>`);
  }
  if (s.paused) {
    banners.push(`<div class="banner"><b>Tracking is paused.</b> <button class="link" data-action="resume">Resume</button></div>`);
  }
  if (!s.tracking) banners.push(`<div class="banner">Read-only dashboard: this instance shows data but does not track.</div>`);

  const legend = r.categories
    .reduce<{ name: string; ms: number }[]>((acc, c) => {
      const g = chartGroup(c.name);
      const hit = acc.find((x) => x.name === g);
      if (hit) hit.ms += c.ms;
      else acc.push({ name: g, ms: c.ms });
      return acc;
    }, [])
    .sort((a, b) => b.ms - a.ms);

  const now = current && s.focus && !s.idle && !s.locked ? `<span class="live"><i></i>Now: ${esc(s.focus)}${s.domain ? ` · ${esc(s.domain)}` : ""}</span>` : "";

  app.innerHTML = `
  <header class="top">
    <div class="brand"><img src="/favicon.svg" alt=""><span>Screen Time</span></div>
    <div class="seg" role="tablist">
      ${(["day", "week", "month"] as View[]).map((v) => `<button role="tab" aria-selected="${state.view === v}" data-view="${v}">${v[0].toUpperCase() + v.slice(1)}</button>`).join("")}
    </div>
    <div class="nav">
      <button class="icon-btn" data-step="-1" aria-label="Previous">‹</button>
      <label class="date-pick"><span>${esc(dates.title(state.view, state.anchor))}</span>
        <input type="date" value="${state.anchor}" max="${dates.today()}" aria-label="Pick a date"></label>
      <button class="icon-btn" data-step="1" aria-label="Next" ${atToday ? "disabled" : ""}>›</button>
      ${current ? "" : `<button class="pill" data-today>Today</button>`}
    </div>
  </header>
  <main>
    ${banners.join("")}
    <div class="section-label">Screen time</div>
    <section class="card hero">
      <div class="hero-head">
        <div>
          <div class="hero-date">${esc(heroDate())}</div>
          <div class="big">${dur(multi ? r.daily_avg_ms : r.total_ms)}</div>
          <div class="sub">${multi ? `Daily average${compareText(r) ? " · " : ""}` : ""}${compareText(r)}${multi ? ` · Total ${dur(r.total_ms)}` : ""}</div>
        </div>
        <div class="hero-side">${now}</div>
      </div>
      ${state.view === "day" ? `<div class="chart-box" id="week-chart"></div>` : ""}
      <div class="chart-box" id="usage-chart"></div>
      <ul class="legend apple">${legend.slice(0, 3).map((c) => `<li data-cat="${esc(c.name)}"><div><i style="background:${catColor(c.name)}"></i>${esc(c.name)}</div><b>${dur(c.ms)}</b></li>`).join("")}</ul>
    </section>
    <div class="updated">${current ? `Updated today at ${clock(Date.now())}` : ""}</div>

    <div class="section-label row-label"><span>Most used</span>
      <label class="popup"><select data-show aria-label="Show">
        ${(["apps", "sites", "categories"] as const).map((t) => `<option value="${t}" ${state.tab === t ? "selected" : ""}>Show ${t === "sites" ? "Websites" : t[0].toUpperCase() + t.slice(1)}</option>`).join("")}
      </select></label>
    </div>
    <section class="card list-card">
      ${mostUsed(r, s)}
    </section>

    <div class="grid">
      <section class="card">
        <div class="card-head"><h2>Pickups</h2><span class="muted">logins and unlocks</span></div>
        <div class="stat-row">
          <div><div class="big2">${count(r.pickups.count)}</div><div class="muted">${multi ? `${(r.pickups.count / Math.max(1, elapsedDays(r))).toFixed(1)} a day` : r.pickups.first_at_ms ? `First at ${clock(r.pickups.first_at_ms)}` : "No pickups yet"}</div></div>
          <div class="mini-stats"><span><b>${r.pickups.logins}</b> logins</span><span><b>${r.pickups.unlocks}</b> unlocks</span></div>
        </div>
        <div class="chart-box" id="pickup-chart"></div>
        <h3>First used after pickup</h3>
        ${listRows("first", r.pickups.first_apps, (a) => countRow(a, r.pickups.first_apps[0].count, "×"), 5)}
      </section>

      <section class="card">
        <div class="card-head"><h2>Notifications</h2></div>
        <div class="stat-row">
          <div><div class="big2">${count(r.notifications.count)}</div><div class="muted">${multi ? `${Math.round(r.notifications.count / Math.max(1, elapsedDays(r)))} a day` : `${r.notifications.apps.length} apps`}</div></div>
        </div>
        <div class="chart-box" id="notif-chart"></div>
        <h3>From</h3>
        ${listRows("notif", r.notifications.apps, (a) => countRow(a, r.notifications.apps[0].count, ""), 5)}
      </section>
    </div>

    <section class="card">
      <div class="card-head"><h2>Network data</h2>
        <div class="ifaces">${r.network.ifaces.map((i) => `<span class="chip">${i.kind === "wifi" ? "Wi-Fi" : "Ethernet"} <small>${esc(i.name)}</small> ${bytes(i.rx + i.tx)}</span>`).join("")}</div>
      </div>
      <div class="stat-row">
        <div><div class="big2">${bytes(r.network.rx + r.network.tx)}</div><div class="muted">↓ ${bytes(r.network.rx)} downloaded · ↑ ${bytes(r.network.tx)} uploaded</div></div>
        <ul class="legend inline"><li><i style="background:var(--net-rx)"></i><span>Download</span></li><li><i style="background:var(--net-tx)"></i><span>Upload</span></li></ul>
      </div>
      <div class="chart-box" id="net-chart"></div>
      <h3>By app</h3>
      ${netList(r)}
      <p class="foot">Per-app figures are measured from TCP connections. UDP and QUIC traffic (common for video calls and some streaming), very short connections and system services show up as <i>Other</i>.</p>
    </section>
  </main>`;

  // Charts
  if (state.view === "day" && weekReport) {
    const w = weekReport;
    const t = dates.today();
    barChart(document.getElementById("week-chart")!, usageColumns(w).map((c, i) => {
      const day = dayOf(i, w);
      const selected = day === state.anchor;
      return {
        ...c,
        highlight: selected,
        parts: selected ? c.parts : [{ name: "Screen time", value: c.parts.reduce((a, p) => a + p.value, 0), color: "var(--bar-muted)" }],
        onClick: day <= t && !selected ? () => go("day", day) : undefined,
      };
    }), {
      height: 130,
      fmt: (v) => (v === 0 ? "0" : dur(v)),
      scale: durationScale,
      reference: w.daily_avg_ms > 0 ? { value: w.daily_avg_ms, label: "avg" } : undefined,
      breakdown: true,
      sparseTicks: true,
    });
  }
  const avgRef = multi && r.daily_avg_ms > 0 ? { value: r.daily_avg_ms, label: "avg" } : undefined;
  barChart(document.getElementById("usage-chart")!, usageColumns(r), {
    height: 200,
    fmt: (v) => (v === 0 ? "0" : r.bucket === "hour" ? durationTick(v) : dur(v)),
    scale: r.bucket === "hour" ? hourScale : durationScale,
    reference: avgRef,
    breakdown: true,
    sparseTicks: multi,
  });
  barChart(document.getElementById("pickup-chart")!, countColumns(r, r.pickups.buckets, "var(--accent)"), {
    height: 120, fmt: (v) => count(v), scale: countScale,
  });
  barChart(document.getElementById("notif-chart")!, countColumns(r, r.notifications.buckets, "var(--accent)"), {
    height: 120, fmt: (v) => count(v), scale: countScale,
  });
  barChart(
    document.getElementById("net-chart")!,
    r.network.buckets.map((b, i) => ({
      tick: tickFor(i, r.buckets[i].label, r),
      heading: bucketHeading(i, r),
      parts: [
        { name: "Download", value: b.rx, color: "var(--net-rx)" },
        { name: "Upload", value: b.tx, color: "var(--net-tx)" },
      ],
    })),
    { height: 150, fmt: (v) => (v === 0 ? "0" : bytes(v)), scale: byteScale, breakdown: true },
  );
}

function elapsedDays(r: Report): number {
  const now = Date.now();
  return r.buckets.filter((b) => b.start_ms <= now).length;
}

function heroDate(): string {
  const t = dates.title(state.view, state.anchor);
  if (state.view !== "day") return t;
  const long = dates.parse(state.anchor).toLocaleDateString([], { day: "numeric", month: "long" });
  return t === "Today" || t === "Yesterday" ? `${t}, ${long}` : t;
}

/** Screen Time row: icon, name, then a thin bar with the value beside it. */
function appleRow(attrs: string, iconHtml: string, name: string, sub: string, fraction: number, color: string, value: string): string {
  return `<li class="row" ${attrs}>${iconHtml}<div class="row-main">
    <div class="name">${esc(name)}${sub ? `<small>${esc(sub)}</small>` : ""}</div>
    <div class="bar-line"><div class="fill" style="--f:${Math.min(1, fraction)};background:${color}"></div><span class="value">${value}</span></div></div><span class="chev">›</span></li>`;
}

function mostUsed(r: Report, s: Status): string {
  const multi = state.view !== "day";
  const avg = (ms: number, a: number) => (multi ? `<small>${dur(a)}/day</small>` : "") + dur(ms);
  if (state.tab === "apps") {
    const max = r.apps[0]?.ms ?? 1;
    return listRows("apps", r.apps, (a) => appleRow(
      `data-app="${esc(a.id)}" data-name="${esc(a.name)}" data-icon="${a.has_icon ? 1 : 0}" data-category="${esc(a.category)}"`,
      icon(a.id, a.name, a.has_icon, a.category), a.name, a.category, a.ms / max, catColor(a.category), avg(a.ms, a.daily_avg_ms)), 8);
  }
  if (state.tab === "sites") {
    if (r.sites.length === 0 && !s.browser_extension) {
      return `<p class="empty">No websites yet. Load the browser extension from <code>browser-extension/</code> (see README) to see time per site.</p>`;
    }
    const max = r.sites[0]?.ms ?? 1;
    return listRows("sites", r.sites, (w) => appleRow(
      `data-domain="${esc(w.domain)}" data-category="${esc(w.category)}"`,
      siteIcon(w.domain, w.category), w.domain, w.category, w.ms / max, catColor(w.category), avg(w.ms, w.daily_avg_ms)), 8);
  }
  const max = r.categories[0]?.ms ?? 1;
  const days = Math.max(1, elapsedDays(r));
  const all = `<li class="row static all"><div class="icon swatch stack"></div><div class="row-main">
    <div class="name">All Categories</div><div class="bar-line"><div class="fill" style="--f:1;background:var(--bar-muted)"></div><span class="value">${avg(r.total_ms, r.daily_avg_ms)}</span></div></div></li>`;
  if (r.categories.length === 0) return `<p class="empty">Nothing recorded yet.</p>`;
  return listRows("cats", r.categories, (c) => appleRow(`data-cat="${esc(c.name)}"`,
    `<div class="icon swatch" style="--tint:${catColor(c.name)}"></div>`, c.name, "", c.ms / max, catColor(c.name), avg(c.ms, Math.round(c.ms / days))), 13)
    .replace('<ul class="rows">', `<ul class="rows">${all}`);
}

function netList(r: Report): string {
  const items = [...r.network.apps];
  const other = r.network.unattributed;
  const rows = items.map((a) => ({ ...a, other: false }));
  if (other.rx + other.tx > 0) rows.push({ id: "", name: "Other", has_icon: false, rx: other.rx, tx: other.tx, other: true });
  rows.sort((a, b) => b.rx + b.tx - (a.rx + a.tx));
  const max = rows[0] ? rows[0].rx + rows[0].tx : 1;
  return listRows("net", rows, (a) => `<li class="row" ${a.other ? "" : `data-app="${esc(a.id)}" data-name="${esc(a.name)}" data-icon="${a.has_icon ? 1 : 0}"`}>
    ${icon(a.id, a.name, a.has_icon)}
    <div class="row-main"><div class="row-top"><span class="name">${esc(a.name)}${a.other ? "<small>not matched to an app</small>" : ""}</span>
    <span class="value"><small>↓ ${bytes(a.rx)}  ↑ ${bytes(a.tx)}</small>${bytes(a.rx + a.tx)}</span></div>
    <div class="meter split"><div style="width:${(a.rx / max) * 100}%;background:var(--net-rx)"></div><div style="width:${(a.tx / max) * 100}%;background:var(--net-tx)"></div></div></div></li>`, 8);
}

// Detail sheet for one app, site or category.

async function openDetail(filter: Filter) {
  const { from, to } = dates.range(state.view, state.anchor);
  const sheet = document.createElement("div");
  sheet.className = "sheet-wrap";
  sheet.innerHTML = `<div class="scrim"></div><aside class="sheet" role="dialog" aria-modal="true"><div class="sheet-body"><p class="muted">Loading…</p></div></aside>`;
  document.body.append(sheet);
  const close = () => (hideTip(), sheet.remove(), document.removeEventListener("keydown", onKey));
  const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
  document.addEventListener("keydown", onKey);
  sheet.querySelector(".scrim")!.addEventListener("click", close);

  const d = await getJson<Report>(reportUrl(from, to, filter));
  const multi = state.view !== "day";
  const color = filter.kind === "category" ? catColor(filter.id) : catColor(filter.category);
  const head =
    filter.kind === "app"
      ? `${icon(filter.id, filter.name, filter.has_icon, filter.category)}<div><h2>${esc(filter.name)}</h2><div class="muted">${esc(filter.category)}</div></div>`
      : filter.kind === "domain"
        ? `${siteIcon(filter.id, filter.category)}<div><h2>${esc(filter.id)}</h2><div class="muted">${esc(filter.category)}</div></div>`
        : `<div class="icon swatch" style="--tint:${color}"></div><div><h2>${esc(filter.id)}</h2><div class="muted">Category</div></div>`;

  const tiles = [
    [multi ? "Daily average" : "Today", dur(multi ? d.daily_avg_ms : d.total_ms)],
    ...(multi ? [["Total", dur(d.total_ms)]] : []),
    ...(filter.kind !== "domain"
      ? [
          ["Notifications", count(d.notifications.count)],
          ["Opened first after pickup", count(d.pickups.count)],
          ["Network data", bytes(d.network.rx + d.network.tx)],
        ]
      : []),
  ];

  const body = sheet.querySelector(".sheet-body")!;
  body.innerHTML = `
    <div class="sheet-head">${head}<button class="icon-btn close" aria-label="Close">✕</button></div>
    <div class="muted">${esc(dates.subtitle(state.view, state.anchor))}</div>
    <div class="tiles">${tiles.map(([k, v]) => `<div class="tile"><div class="muted">${k}</div><div class="tile-v">${v}</div></div>`).join("")}</div>
    <div class="chart-box" id="detail-chart"></div>
    ${filter.kind !== "domain" && d.sites.length ? `<h3>Websites</h3>${listRows("d-sites", d.sites, (w) => `<li class="row static">${siteIcon(w.domain, w.category)}<div class="row-main"><div class="row-top"><span class="name">${esc(w.domain)}</span><span class="value">${dur(w.ms)}</span></div>${meter(w.ms / d.sites[0].ms, catColor(w.category))}</div></li>`, 6)}` : ""}
    ${filter.kind === "category" && d.apps.length ? `<h3>Apps</h3>${listRows("d-apps", d.apps, (a) => `<li class="row static">${icon(a.id, a.name, a.has_icon, a.category)}<div class="row-main"><div class="row-top"><span class="name">${esc(a.name)}</span><span class="value">${dur(a.ms)}</span></div>${meter(a.ms / d.apps[0].ms, catColor(a.category))}</div></li>`, 6)}` : ""}
  `;
  body.querySelector(".close")!.addEventListener("click", close);
  barChart(body.querySelector<HTMLElement>("#detail-chart")!, usageColumns(d, color).map((c) => ({ ...c, onClick: undefined })), {
    height: 170,
    fmt: (v) => (v === 0 ? "0" : d.bucket === "hour" ? durationTick(v) : dur(v)),
    scale: d.bucket === "hour" ? hourScale : durationScale,
    reference: multi && d.daily_avg_ms > 0 ? { value: d.daily_avg_ms, label: "avg" } : undefined,
  });
}

// Events (delegated, since render() replaces the DOM).

app.addEventListener("click", async (e) => {
  const t = e.target as HTMLElement;
  const btn = t.closest<HTMLElement>("[data-view],[data-step],[data-today],[data-tab],[data-expand],[data-action]");
  if (btn) {
    if (btn.dataset.view) go(btn.dataset.view as View, state.anchor);
    else if (btn.dataset.step) {
      let next = dates.step(state.view, state.anchor, Number(btn.dataset.step) as 1 | -1);
      if (next > dates.today()) next = dates.today();
      go(state.view, next);
    } else if (btn.dataset.today !== undefined) go(state.view, dates.today());
    else if (btn.dataset.tab) {
      state.tab = btn.dataset.tab as State["tab"];
      render();
    } else if (btn.dataset.expand) {
      state.expanded[btn.dataset.expand] = !state.expanded[btn.dataset.expand];
      render();
    } else if (btn.dataset.action === "resume") {
      await fetch("/api/pause", { method: "POST", headers: { "content-type": "application/json", "x-screentime": "1" }, body: JSON.stringify({ paused: false }) });
      load();
    }
    return;
  }
  const row = t.closest<HTMLElement>("[data-app],[data-domain],[data-cat]");
  if (!row) return;
  const ds = row.dataset;
  if (ds.app) openDetail({ kind: "app", id: ds.app, name: ds.name ?? ds.app, has_icon: ds.icon === "1", category: ds.category ?? "" });
  else if (ds.domain) openDetail({ kind: "domain", id: ds.domain, category: ds.category ?? "" });
  else if (ds.cat) openDetail({ kind: "category", id: ds.cat });
});

app.addEventListener("change", (e) => {
  const input = e.target as HTMLInputElement;
  if (input.type === "date" && input.value) go(state.view, input.value);
  if (input.dataset.show !== undefined) {
    state.tab = input.value as State["tab"];
    render();
  }
});

// Keyboard: arrows move through dates, d/w/m switch views.
document.addEventListener("keydown", (e) => {
  if (document.querySelector(".sheet-wrap") || (e.target as HTMLElement).tagName === "INPUT") return;
  if (e.key === "ArrowLeft") go(state.view, dates.step(state.view, state.anchor, -1));
  else if (e.key === "ArrowRight" && dates.range(state.view, state.anchor).to < dates.today()) go(state.view, dates.step(state.view, state.anchor, 1));
  else if (e.key === "d" || e.key === "w" || e.key === "m") go(({ d: "day", w: "week", m: "month" } as const)[e.key], state.anchor);
});

// Live refresh while looking at a range that includes today.
setInterval(() => {
  if (isCurrent() && !document.hidden && !document.querySelector(".sheet-wrap")) load();
}, 60_000);

readHash();
load();

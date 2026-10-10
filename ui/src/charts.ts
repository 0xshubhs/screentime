// Hand-rolled SVG bar charts. Spec: bars <= 24px wide, 4px rounded top and square at the
// baseline, 2px surface gaps between stacked segments, hairline solid gridlines, a hover
// tooltip per column with a hit target the full height of the plot.

import { esc } from "./format";

export interface Part {
  name: string;
  value: number;
  color: string;
}

export interface Column {
  /** Axis label, or "" to leave this column unlabeled. */
  tick: string;
  parts: Part[];
  /** Tooltip heading, e.g. "14:00 to 15:00". */
  heading: string;
  onClick?: () => void;
  highlight?: boolean;
}

export interface ChartOptions {
  height?: number;
  /** Formats axis ticks and tooltip values. */
  fmt: (v: number) => string;
  /** Picks a rounded axis maximum and tick step for the data maximum. */
  scale: (max: number) => { max: number; step: number };
  /** Optional horizontal reference line (e.g. daily average). */
  reference?: { value: number; label: string };
  /** Show per-part rows in the tooltip (stacked charts). */
  breakdown?: boolean;
  /** Only label the top gridline and zero, as Screen Time does. */
  sparseTicks?: boolean;
}

const SVG = "http://www.w3.org/2000/svg";
const GAP = 2;

/** Rect with only the top corners rounded. */
function barPath(x: number, y: number, w: number, h: number, r: number): string {
  r = Math.min(r, w / 2, h);
  return `M${x},${y + h}V${y + r}Q${x},${y} ${x + r},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${y + h}Z`;
}

/** Last values drawn per chart (by element id), so the next draw can animate from them. */
const previous = new Map<string, { values: number[][]; max: number; ref: number }>();
const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)");
const DURATION = 700;
const ease = (t: number) => 1 - (1 - t) ** 3;

type ChartEl = HTMLElement & { _ro?: ResizeObserver; _raf?: number; _w?: number };

/**
 * Draws (or redraws) a bar chart. When the same chart (same element id) is drawn again with
 * new numbers, bars and the axis glide from the old values to the new ones; a new chart
 * grows its bars from zero. Unchanged data redraws without motion.
 */
export function barChart(host: HTMLElement, cols: Column[], opt: ChartOptions): void {
  const el = host as ChartEl;
  const target = cols.map((c) => c.parts.map((p) => p.value));
  const dataMax = Math.max(0, ...target.map((r) => r.reduce((a, v) => a + v, 0)), opt.reference?.value ?? 0);
  const { max, step } = opt.scale(dataMax);
  const ref = opt.reference?.value ?? 0;

  const prev = el.id ? previous.get(el.id) : undefined;
  const sameShape = !!prev && prev.values.length === target.length && prev.values.every((r, i) => r.length === target[i].length);
  const from = sameShape ? prev! : { values: target.map((r) => r.map(() => 0)), max, ref };
  const changed =
    !sameShape || from.max !== max || from.ref !== ref || target.some((r, i) => r.some((v, j) => v !== from.values[i][j]));
  if (el.id) previous.set(el.id, { values: target, max, ref });

  const draw = (t: number) => {
    const lerp = (a: number, b: number) => a + (b - a) * t;
    const width = Math.max(240, el.clientWidth);
    el._w = width;
    const height = opt.height ?? 180;
    const pad = { top: 8, right: 44, bottom: 22, left: 0 };
    const plotW = width - pad.left - pad.right;
    const plotH = height - pad.top - pad.bottom;
    const scaleMax = lerp(from.max, max);
    const y = (v: number) => pad.top + plotH - (v / scaleMax) * plotH;
    const band = plotW / Math.max(1, cols.length);
    const barW = Math.max(3, Math.min(24, band * 0.62));

    const svg = document.createElementNS(SVG, "svg");
    svg.setAttribute("width", String(width));
    svg.setAttribute("height", String(height));
    svg.setAttribute("class", "chart");
    let html = "";

    for (let v = 0; v <= max + 1e-9; v += step) {
      if (v > scaleMax + 1e-9) break; // gridlines above the axis appear as it grows into them
      const yy = Math.round(y(v)) + 0.5;
      const edge = v === 0 || v + step > max + 1e-9;
      html += `<line class="grid" x1="${pad.left}" x2="${pad.left + plotW}" y1="${yy}" y2="${yy}"/>`;
      if (!opt.sparseTicks || edge) {
        html += `<text class="axis" x="${pad.left + plotW + 6}" y="${yy + 4}">${esc(opt.fmt(v))}</text>`;
      }
    }
    // Vertical day/hour separators, as in Screen Time.
    cols.forEach((c, i) => {
      if (i > 0 && (cols.length <= 7 || c.tick)) {
        const xx = Math.round(pad.left + band * i) + 0.5;
        html += `<line class="vgrid" x1="${xx}" x2="${xx}" y1="${pad.top}" y2="${pad.top + plotH + 14}"/>`;
      }
    });

    cols.forEach((c, i) => {
      const cx = pad.left + band * i + band / 2;
      const x = cx - barW / 2;
      let base = y(0);
      const visible = c.parts
        .map((p, j) => ({ color: p.color, value: lerp(from.values[i][j], target[i][j]) }))
        .filter((p) => p.value > 0);
      visible.forEach((p, j) => {
        const h = (p.value / scaleMax) * plotH;
        const top = j === visible.length - 1;
        // Leave a surface gap above every segment that has another stacked on it.
        const drawH = Math.max(0, top ? h : h - GAP);
        if (drawH > 0.3) {
          html += top
            ? `<path class="bar" d="${barPath(x, base - drawH, barW, drawH, 4)}" style="fill:${p.color}"/>`
            : `<rect class="bar" x="${x}" y="${base - drawH}" width="${barW}" height="${drawH}" style="fill:${p.color}"/>`;
        }
        base -= h;
      });
      if (c.tick) {
        // Screen Time left-aligns axis labels just after their separator.
        const lx = cols.length <= 7 ? cx : pad.left + band * i + 3;
        const anchor = cols.length <= 7 ? "middle" : "start";
        html += `<text class="axis${c.highlight ? " strong" : ""}" x="${lx}" y="${height - 6}" text-anchor="${anchor}">${esc(c.tick)}</text>`;
      }
      html += `<rect class="hit${c.onClick ? " clickable" : ""}" data-i="${i}" x="${pad.left + band * i}" y="${pad.top}" width="${band}" height="${plotH}"/>`;
    });

    const refNow = lerp(from.ref, ref);
    if (opt.reference && refNow > 0) {
      const yy = Math.round(y(refNow)) + 0.5;
      html += `<line class="ref" x1="${pad.left}" x2="${pad.left + plotW}" y1="${yy}" y2="${yy}"/>`;
      html += `<text class="ref-label" x="${pad.left + plotW + 6}" y="${yy + 4}">${esc(opt.reference.label)}</text>`;
    }

    svg.innerHTML = html;
    svg.querySelectorAll<SVGRectElement>(".hit").forEach((r) => {
      const c = cols[Number(r.dataset.i)];
      r.addEventListener("pointerenter", (e) => showTip(e, tipHtml(c, opt)));
      r.addEventListener("pointermove", moveTip);
      r.addEventListener("pointerleave", hideTip);
      if (c.onClick) r.addEventListener("click", () => (hideTip(), c.onClick!()));
    });
    el.replaceChildren(svg);
  };
  if (el._raf) cancelAnimationFrame(el._raf);
  if (changed && !reduceMotion.matches) {
    const start = performance.now();
    const frame = (now: number) => {
      const t = Math.min(1, (now - start) / DURATION);
      draw(ease(t));
      el._raf = t < 1 ? requestAnimationFrame(frame) : undefined;
    };
    draw(0);
    el._raf = requestAnimationFrame(frame);
  } else {
    draw(1);
  }
  // Redraw (without motion) only when the width really changes.
  el._ro?.disconnect();
  el._ro = new ResizeObserver(() => {
    if (Math.max(240, el.clientWidth) !== el._w && !el._raf) draw(1);
  });
  el._ro.observe(el);
}

/** Tweens the text of `[data-num]` elements from their last value to the new one. */
const lastNumbers = new Map<string, number>();
export function tweenNumbers(root: ParentNode, fmt: Record<string, (v: number) => string>): void {
  root.querySelectorAll<HTMLElement>("[data-num]").forEach((el) => {
    const [key, kind, raw] = el.dataset.num!.split("|");
    const to = Number(raw);
    const from = lastNumbers.get(key);
    lastNumbers.set(key, to);
    const f = fmt[kind];
    if (from === undefined || from === to || reduceMotion.matches || !f) return;
    const start = performance.now();
    const frame = (now: number) => {
      const t = Math.min(1, (now - start) / DURATION);
      el.textContent = f(from + (to - from) * ease(t));
      if (t < 1 && el.isConnected) requestAnimationFrame(frame);
    };
    el.textContent = f(from);
    requestAnimationFrame(frame);
  });
}

/**
 * List bars (`[data-k]`) slide from their previous length to the new one. Called with the
 * styles captured before a re-render; new rows grow from zero.
 */
export function captureBars(root: ParentNode): Map<string, string> {
  const m = new Map<string, string>();
  root.querySelectorAll<HTMLElement>("[data-k]").forEach((e) => m.set(e.dataset.k!, e.style.cssText));
  return m;
}

export function slideBars(root: ParentNode, before: Map<string, string>): void {
  if (reduceMotion.matches) return;
  const els = [...root.querySelectorAll<HTMLElement>("[data-k]")];
  const targets = els.map((e) => e.style.cssText);
  els.forEach((e) => {
    const old = before.get(e.dataset.k!);
    e.style.cssText = old ?? e.style.cssText.replace(/--f:\s*[\d.]+/, "--f:0").replace(/width:\s*[\d.]+%/, "width:0%");
  });
  void (root as HTMLElement).offsetWidth; // commit the old lengths so the change transitions
  els.forEach((e, i) => (e.style.cssText = targets[i]));
}

function tipHtml(c: Column, opt: ChartOptions): string {
  const total = c.parts.reduce((s, p) => s + p.value, 0);
  let h = `<div class="tip-head">${esc(c.heading)}</div><div class="tip-total">${esc(opt.fmt(total))}</div>`;
  if (opt.breakdown) {
    const rows = c.parts.filter((p) => p.value > 0).sort((a, b) => b.value - a.value);
    h += rows
      .map((p) => `<div class="tip-row"><i style="background:${p.color}"></i><span>${esc(p.name)}</span><b>${esc(opt.fmt(p.value))}</b></div>`)
      .join("");
  }
  if (c.onClick) h += `<div class="tip-hint">Click to open this day</div>`;
  return h;
}

const tip = () => document.getElementById("tip")!;

function showTip(e: PointerEvent, html: string) {
  const t = tip();
  t.innerHTML = html;
  t.hidden = false;
  moveTip(e);
}

function moveTip(e: PointerEvent) {
  const t = tip();
  const r = t.getBoundingClientRect();
  let x = e.clientX + 14;
  let y = e.clientY + 14;
  if (x + r.width > window.innerWidth - 8) x = e.clientX - r.width - 14;
  if (y + r.height > window.innerHeight - 8) y = e.clientY - r.height - 14;
  t.style.transform = `translate(${Math.max(8, x)}px, ${Math.max(8, y)}px)`;
}

export function hideTip() {
  tip().hidden = true;
}

// Axis scales.

const MIN = 60_000;
const HOUR = 60 * MIN;

export function durationScale(max: number) {
  const steps = [5 * MIN, 10 * MIN, 15 * MIN, 30 * MIN, HOUR, 2 * HOUR, 3 * HOUR, 4 * HOUR, 6 * HOUR, 12 * HOUR];
  const s = steps.find((st) => st * 3 >= max) ?? 24 * HOUR;
  return { max: Math.max(s, Math.ceil(max / s) * s), step: s };
}

/** Hourly view: always 0 to 60m, like Screen Time. */
export function hourScale(max: number) {
  return max <= HOUR ? { max: HOUR, step: 30 * MIN } : durationScale(max);
}

export function countScale(max: number) {
  if (max <= 4) return { max: 4, step: 2 };
  const raw = max / 3;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const s = [1, 2, 5, 10].map((m) => m * mag).find((v) => v >= raw)!;
  return { max: Math.ceil(max / s) * s, step: s };
}

export function byteScale(max: number) {
  if (max <= 0) return { max: 1_000_000, step: 500_000 };
  return countScale(max);
}

export function durationTick(v: number): string {
  if (v === 0) return "0";
  if (v < HOUR) return `${Math.round(v / MIN)}m`;
  return `${+(v / HOUR).toFixed(1)}h`;
}

/** Horizontal meter used in list rows. */
/** Horizontal meter used in list rows; `key` lets it slide between refreshes. */
export function meter(fraction: number, color: string, key?: string): string {
  const w = Math.max(fraction > 0 ? 1.5 : 0, Math.min(100, fraction * 100));
  const k = key ? ` data-k="${esc(key)}"` : "";
  return `<div class="meter"><div${k} style="width:${w}%;background:${color}"></div></div>`;
}

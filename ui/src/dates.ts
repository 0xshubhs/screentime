import type { View } from "./types";

// Dates are local calendar days, carried around as YYYY-MM-DD strings.

export function ymd(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

export function parse(s: string): Date {
  const [y, m, d] = s.split("-").map(Number);
  return new Date(y, m - 1, d);
}

export function today(): string {
  return ymd(new Date());
}

export function addDays(s: string, n: number): string {
  const d = parse(s);
  d.setDate(d.getDate() + n);
  return ymd(d);
}

/** Weeks start on Monday. */
export function range(view: View, anchor: string): { from: string; to: string } {
  const d = parse(anchor);
  if (view === "day") return { from: anchor, to: anchor };
  if (view === "week") {
    const offset = (d.getDay() + 6) % 7;
    const from = addDays(anchor, -offset);
    return { from, to: addDays(from, 6) };
  }
  const from = new Date(d.getFullYear(), d.getMonth(), 1);
  const to = new Date(d.getFullYear(), d.getMonth() + 1, 0);
  return { from: ymd(from), to: ymd(to) };
}

export function step(view: View, anchor: string, dir: 1 | -1): string {
  if (view === "day") return addDays(anchor, dir);
  if (view === "week") return addDays(anchor, 7 * dir);
  const d = parse(anchor);
  return ymd(new Date(d.getFullYear(), d.getMonth() + dir, 1));
}

export function title(view: View, anchor: string): string {
  const t = today();
  const { from, to } = range(view, anchor);
  if (view === "day") {
    if (anchor === t) return "Today";
    if (anchor === addDays(t, -1)) return "Yesterday";
    return parse(anchor).toLocaleDateString([], { weekday: "long", day: "numeric", month: "long" });
  }
  if (view === "week") {
    if (from <= t && t <= to) return "This week";
    const f = parse(from).toLocaleDateString([], { day: "numeric", month: "short" });
    const e = parse(to).toLocaleDateString([], { day: "numeric", month: "short", year: "numeric" });
    return `${f} to ${e}`;
  }
  return parse(from).toLocaleDateString([], { month: "long", year: "numeric" });
}

export function subtitle(view: View, anchor: string): string {
  if (view === "day") return parse(anchor).toLocaleDateString([], { weekday: "long", day: "numeric", month: "long", year: "numeric" });
  const { from, to } = range(view, anchor);
  const f = parse(from).toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" });
  const e = parse(to).toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" });
  return `${f} to ${e}`;
}

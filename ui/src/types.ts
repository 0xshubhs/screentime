// Mirrors screentime_core::report::Report.

export interface Bucket {
  start_ms: number;
  label: string;
  total_ms: number;
  categories: Record<string, number>;
}

export interface AppUsage {
  id: string;
  name: string;
  has_icon: boolean;
  category: string;
  ms: number;
  daily_avg_ms: number;
}

export interface SiteUsage {
  domain: string;
  category: string;
  ms: number;
  daily_avg_ms: number;
}

export interface AppCount {
  id: string;
  name: string;
  has_icon: boolean;
  count: number;
}

export interface Bytes {
  rx: number;
  tx: number;
}

export interface NetApp extends Bytes {
  id: string;
  name: string;
  has_icon: boolean;
}

export interface NetIface extends Bytes {
  name: string;
  kind: "wifi" | "ethernet" | string;
}

export interface Report {
  from: string;
  to: string;
  bucket: "hour" | "day";
  buckets: Bucket[];
  total_ms: number;
  daily_avg_ms: number;
  compare_avg_ms: number | null;
  categories: { name: string; ms: number }[];
  apps: AppUsage[];
  sites: SiteUsage[];
  pickups: {
    count: number;
    logins: number;
    unlocks: number;
    first_at_ms: number | null;
    buckets: number[];
    first_apps: AppCount[];
  };
  notifications: { count: number; buckets: number[]; apps: AppCount[] };
  network: {
    rx: number;
    tx: number;
    buckets: Bytes[];
    apps: NetApp[];
    ifaces: NetIface[];
    unattributed: Bytes;
  };
}

export interface Status {
  version: string;
  tracking: boolean;
  paused: boolean;
  extension: boolean;
  browser_extension: boolean;
  focus: string | null;
  domain: string | null;
  idle: boolean;
  locked: boolean;
}

export type View = "day" | "week" | "month";

export type Filter =
  | { kind: "app"; id: string; name: string; has_icon: boolean; category: string }
  | { kind: "domain"; id: string; category: string }
  | { kind: "category"; id: string };

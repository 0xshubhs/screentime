//! Turns raw rows into the numbers the UI and CLI show: totals, per-bucket stacks, top apps,
//! sites and categories, pickups, notifications and network data.

use std::cmp::Reverse;
use std::collections::HashMap;

use chrono::{Local, NaiveDate, TimeZone, Timelike};
use serde::Serialize;

use crate::apps::AppIndex;
use crate::db::{Db, Kind, Result, UsageRow};
use crate::Rules;

const HOUR_MS: i64 = 3_600_000;

/// What the report is narrowed to.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Filter {
    #[default]
    None,
    App(String),
    Domain(String),
    Category(String),
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// "hour" for a single day, "day" otherwise.
    pub bucket: &'static str,
    pub buckets: Vec<Bucket>,
    pub total_ms: i64,
    pub daily_avg_ms: i64,
    /// Daily average of the comparison period (previous 7 days for a day, previous period
    /// otherwise), counting only days that have data.
    pub compare_avg_ms: Option<i64>,
    pub categories: Vec<Named>,
    pub apps: Vec<AppUsage>,
    pub sites: Vec<SiteUsage>,
    pub pickups: Pickups,
    pub notifications: Notifications,
    pub network: Network,
}

#[derive(Debug, Serialize)]
pub struct Bucket {
    pub start_ms: i64,
    pub label: String,
    pub total_ms: i64,
    pub categories: HashMap<String, i64>,
}

#[derive(Debug, Serialize)]
pub struct Named {
    pub name: String,
    pub ms: i64,
}

#[derive(Debug, Serialize)]
pub struct AppUsage {
    pub id: String,
    pub name: String,
    pub has_icon: bool,
    pub category: String,
    pub ms: i64,
    pub daily_avg_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct SiteUsage {
    pub domain: String,
    pub category: String,
    pub ms: i64,
    pub daily_avg_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct AppCount {
    pub id: String,
    pub name: String,
    pub has_icon: bool,
    pub count: i64,
}

#[derive(Debug, Serialize, Default)]
pub struct Pickups {
    pub count: i64,
    pub logins: i64,
    pub unlocks: i64,
    pub first_at_ms: Option<i64>,
    pub buckets: Vec<i64>,
    pub first_apps: Vec<AppCount>,
}

#[derive(Debug, Serialize, Default)]
pub struct Notifications {
    pub count: i64,
    pub buckets: Vec<i64>,
    pub apps: Vec<AppCount>,
}

#[derive(Debug, Serialize, Default, Clone, Copy)]
pub struct Bytes {
    pub rx: u64,
    pub tx: u64,
}

#[derive(Debug, Serialize)]
pub struct NetApp {
    pub id: String,
    pub name: String,
    pub has_icon: bool,
    pub rx: u64,
    pub tx: u64,
}

#[derive(Debug, Serialize)]
pub struct NetIface {
    pub name: String,
    pub kind: String,
    pub rx: u64,
    pub tx: u64,
}

#[derive(Debug, Serialize, Default)]
pub struct Network {
    pub rx: u64,
    pub tx: u64,
    pub buckets: Vec<Bytes>,
    pub apps: Vec<NetApp>,
    pub ifaces: Vec<NetIface>,
    /// Interface traffic not matched to an app (UDP/QUIC, short connections, system services).
    pub unattributed: Bytes,
}

/// Local midnight at the start of `d`, in Unix ms.
pub fn day_start(d: NaiveDate) -> i64 {
    let naive = d.and_hms_opt(0, 0, 0).expect("midnight exists");
    Local
        .from_local_datetime(&naive)
        .earliest()
        .unwrap_or_else(|| Local.from_utc_datetime(&naive))
        .timestamp_millis()
}

fn day_end(d: NaiveDate) -> i64 {
    day_start(d.succ_opt().expect("date in range"))
}

struct Ctx<'a> {
    rules: &'a Rules,
    apps: &'a AppIndex,
    filter: &'a Filter,
}

impl Ctx<'_> {
    fn app_cat(&self, id: &str) -> &str {
        self.rules.categorize_app(id)
    }

    fn keep_app_id(&self, id: &str) -> bool {
        match self.filter {
            Filter::None => true,
            Filter::App(a) => a == id,
            Filter::Category(c) => self.app_cat(id) == c,
            Filter::Domain(_) => false,
        }
    }

    fn app_count(&self, counts: HashMap<String, i64>) -> Vec<AppCount> {
        let mut v: Vec<_> = counts
            .into_iter()
            .map(|(id, count)| {
                let info = self.apps.info(&id);
                AppCount { has_icon: self.apps.icon_path(&id).is_some(), name: info.name, id, count }
            })
            .collect();
        v.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
        v
    }
}

/// Adds the part of [start, end) that falls in each bucket.
fn spread(bounds: &[i64], start: i64, end: i64, mut f: impl FnMut(usize, i64)) {
    let first = bounds.partition_point(|&b| b <= start).saturating_sub(1);
    for i in first..bounds.len() - 1 {
        let (b0, b1) = (bounds[i], bounds[i + 1]);
        if b0 >= end {
            break;
        }
        let d = end.min(b1) - start.max(b0);
        if d > 0 {
            f(i, d);
        }
    }
}

fn bucket_of(bounds: &[i64], ts: i64) -> Option<usize> {
    if ts < bounds[0] || ts >= *bounds.last()? {
        return None;
    }
    Some(bounds.partition_point(|&b| b <= ts) - 1)
}

pub fn build(
    db: &Db,
    rules: &Rules,
    apps: &AppIndex,
    from: NaiveDate,
    to: NaiveDate,
    filter: &Filter,
    now_ms: i64,
) -> Result<Report> {
    let ctx = Ctx { rules, apps, filter };
    let single = from == to;
    let (start, end) = (day_start(from), day_end(to));

    // Bucket boundaries and labels.
    let mut bounds = vec![start];
    let mut labels = Vec::new();
    if single {
        while *bounds.last().unwrap() < end {
            let b = *bounds.last().unwrap();
            labels.push(format!("{:02}", Local.timestamp_millis_opt(b).unwrap().hour()));
            bounds.push((b + HOUR_MS).min(end));
        }
    } else {
        let mut d = from;
        while d <= to {
            labels.push(d.format("%a %-d").to_string());
            d = d.succ_opt().unwrap();
            bounds.push(day_start(d));
        }
    }
    let n = labels.len();
    let mut buckets: Vec<Bucket> = labels
        .into_iter()
        .enumerate()
        .map(|(i, label)| Bucket { start_ms: bounds[i], label, total_ms: 0, categories: HashMap::new() })
        .collect();

    // Days that have started, for daily averages.
    let elapsed_days = if single {
        1
    } else {
        (0..n).filter(|&i| bounds[i] <= now_ms).count().max(1) as i64
    };

    // Usage.
    let rows = db.usage_between(start, end)?;
    let mut app_ms: HashMap<String, i64> = HashMap::new();
    let mut site_ms: HashMap<String, i64> = HashMap::new();
    let cat_filter = match filter {
        Filter::Category(c) => Some(c.as_str()),
        _ => None,
    };
    let rows: Vec<&UsageRow> = rows.iter().filter(|r| !crate::is_ignored_app(&r.app_id)).collect();
    for r in rows.iter().filter(|r| r.kind != Kind::Background) {
        let (s, e) = (r.start_ms.max(start), r.end_ms.min(end));
        if r.kind == Kind::Web {
            let domain = r.domain.as_deref().unwrap_or_default();
            let site_cat = rules.categorize_domain(domain).to_string();
            let browser_cat = ctx.app_cat(&r.app_id).to_string();
            match filter {
                // Browser time on a known site moves from the browser's category to the site's.
                Filter::None | Filter::Category(_) => spread(&bounds, s, e, |i, d| {
                    let cats = &mut buckets[i].categories;
                    *cats.entry(browser_cat.clone()).or_default() -= d;
                    *cats.entry(site_cat.clone()).or_default() += d;
                }),
                Filter::App(a) if *a == r.app_id => spread(&bounds, s, e, |i, d| {
                    let cats = &mut buckets[i].categories;
                    *cats.entry(browser_cat.clone()).or_default() -= d;
                    *cats.entry(site_cat.clone()).or_default() += d;
                }),
                Filter::Domain(dm) if dm == domain => spread(&bounds, s, e, |i, d| {
                    buckets[i].total_ms += d;
                    *buckets[i].categories.entry(site_cat.clone()).or_default() += d;
                }),
                _ => continue,
            }
            if cat_filter.is_none_or(|c| c == site_cat) {
                *site_ms.entry(domain.to_string()).or_default() += e - s;
            }
        } else {
            let cat = ctx.app_cat(&r.app_id).to_string();
            let keep = match filter {
                Filter::None | Filter::Category(_) => true,
                Filter::App(a) => *a == r.app_id,
                Filter::Domain(_) => false,
            };
            if !keep {
                continue;
            }
            spread(&bounds, s, e, |i, d| {
                buckets[i].total_ms += d;
                *buckets[i].categories.entry(cat.clone()).or_default() += d;
            });
            if cat_filter.is_none_or(|c| c == cat) {
                *app_ms.entry(r.app_id.clone()).or_default() += e - s;
            }
        }
    }

    // Background use: media playing in a window that is not focused (a video in VLC while
    // browsing), or the top window on another display. It counts toward its app, and toward
    // totals only where no other app was in use, so the same minute is never counted twice.
    let focused: Vec<(i64, i64, &str)> = rows
        .iter()
        .filter(|r| r.kind == Kind::App)
        .map(|r| (r.start_ms.max(start), r.end_ms.min(end), r.app_id.as_str()))
        .collect();
    let mut counted_until = i64::MIN;
    for m in rows.iter().filter(|r| r.kind == Kind::Background) {
        let keep = match filter {
            Filter::None | Filter::Category(_) => true,
            Filter::App(a) => *a == m.app_id,
            Filter::Domain(_) => false,
        };
        if !keep {
            continue;
        }
        let cat = ctx.app_cat(&m.app_id).to_string();
        for (s, e, owner) in uncovered_by(&focused, m.start_ms.max(start), m.end_ms.min(end)) {
            if owner == Some(m.app_id.as_str()) {
                continue; // already counted as focused time
            }
            if cat_filter.is_none_or(|c| c == cat) {
                *app_ms.entry(m.app_id.clone()).or_default() += e - s;
            }
            // For one app's view, its background playback is its time even under another app.
            if owner.is_none() || matches!(filter, Filter::App(_)) {
                let s = s.max(counted_until);
                if s < e {
                    counted_until = e;
                    spread(&bounds, s, e, |i, d| {
                        buckets[i].total_ms += d;
                        *buckets[i].categories.entry(cat.clone()).or_default() += d;
                    });
                }
            }
        }
    }

    for b in &mut buckets {
        b.categories.retain(|k, v| *v > 0 && cat_filter.is_none_or(|c| c == k));
        if cat_filter.is_some() {
            b.total_ms = b.categories.values().sum();
        }
    }
    let total_ms: i64 = buckets.iter().map(|b| b.total_ms).sum();

    let mut cat_ms: HashMap<String, i64> = HashMap::new();
    for b in &buckets {
        for (c, v) in &b.categories {
            *cat_ms.entry(c.clone()).or_default() += v;
        }
    }
    let mut categories: Vec<Named> = cat_ms.into_iter().map(|(name, ms)| Named { name, ms }).collect();
    categories.sort_by_key(|x| Reverse(x.ms));

    let mut apps_out: Vec<AppUsage> = app_ms
        .into_iter()
        .map(|(id, ms)| {
            let info = apps.info(&id);
            AppUsage {
                has_icon: apps.icon_path(&id).is_some(),
                category: ctx.app_cat(&id).to_string(),
                name: info.name,
                id,
                ms,
                daily_avg_ms: ms / elapsed_days,
            }
        })
        .collect();
    apps_out.sort_by_key(|x| Reverse(x.ms));

    let mut sites: Vec<SiteUsage> = site_ms
        .into_iter()
        .map(|(domain, ms)| SiteUsage {
            category: rules.categorize_domain(&domain).to_string(),
            domain,
            ms,
            daily_avg_ms: ms / elapsed_days,
        })
        .collect();
    sites.sort_by_key(|x| Reverse(x.ms));

    // Comparison period (whole-app totals only; filters do not apply).
    let compare_avg_ms = if *filter == Filter::None {
        let days = if single { 7 } else { (to - from).num_days() + 1 };
        let mut totals = Vec::new();
        let mut d = from;
        for _ in 0..days {
            d = d.pred_opt().unwrap();
            totals.push(screen_total(db, day_start(d), day_end(d))?);
        }
        let with_data: Vec<i64> = totals.into_iter().filter(|&t| t > 0).collect();
        (!with_data.is_empty()).then(|| with_data.iter().sum::<i64>() / with_data.len() as i64)
    } else {
        None
    };

    // Pickups: logins and unlocks, and the first app used after each.
    let mut pickups = Pickups { buckets: vec![0; n], ..Default::default() };
    let mut first_apps: HashMap<String, i64> = HashMap::new();
    for (ts, kind, first) in db.pickups_between(start, end)? {
        let keep = match filter {
            Filter::None => true,
            Filter::Domain(_) => false,
            _ => first.as_deref().is_some_and(|a| ctx.keep_app_id(a)),
        };
        if !keep {
            continue;
        }
        pickups.count += 1;
        if kind == "login" {
            pickups.logins += 1;
        } else {
            pickups.unlocks += 1;
        }
        pickups.first_at_ms.get_or_insert(ts);
        if let Some(i) = bucket_of(&bounds, ts) {
            pickups.buckets[i] += 1;
        }
        if let Some(a) = first {
            *first_apps.entry(a).or_default() += 1;
        }
    }
    pickups.first_apps = ctx.app_count(first_apps);

    // Notifications.
    let mut notifications = Notifications { buckets: vec![0; n], ..Default::default() };
    let mut notif_apps: HashMap<String, i64> = HashMap::new();
    for (ts, app) in db.notifications_between(start, end)? {
        if !ctx.keep_app_id(&app) {
            continue;
        }
        notifications.count += 1;
        if let Some(i) = bucket_of(&bounds, ts) {
            notifications.buckets[i] += 1;
        }
        *notif_apps.entry(app).or_default() += 1;
    }
    notifications.apps = ctx.app_count(notif_apps);

    // Network. Totals come from physical interfaces; per-app figures are a TCP estimate.
    let mut network = Network { buckets: vec![Bytes::default(); n], ..Default::default() };
    let mut net_apps: HashMap<String, Bytes> = HashMap::new();
    let mut app_sum = Bytes::default();
    for (slot, app, rx, tx) in db.net_apps_between(start, end)? {
        if !ctx.keep_app_id(&app) {
            continue;
        }
        let e = net_apps.entry(app).or_default();
        e.rx += rx;
        e.tx += tx;
        app_sum.rx += rx;
        app_sum.tx += tx;
        if *filter != Filter::None {
            if let Some(i) = bucket_of(&bounds, slot) {
                network.buckets[i].rx += rx;
                network.buckets[i].tx += tx;
            }
        }
    }
    if *filter == Filter::None {
        let mut ifaces: HashMap<String, NetIface> = HashMap::new();
        for (slot, iface, kind, rx, tx) in db.net_ifaces_between(start, end)? {
            let e = ifaces
                .entry(iface.clone())
                .or_insert_with(|| NetIface { name: iface, kind, rx: 0, tx: 0 });
            e.rx += rx;
            e.tx += tx;
            network.rx += rx;
            network.tx += tx;
            if let Some(i) = bucket_of(&bounds, slot) {
                network.buckets[i].rx += rx;
                network.buckets[i].tx += tx;
            }
        }
        network.ifaces = ifaces.into_values().collect();
        network.ifaces.sort_by_key(|x| Reverse(x.rx + x.tx));
        network.unattributed = Bytes {
            rx: network.rx.saturating_sub(app_sum.rx),
            tx: network.tx.saturating_sub(app_sum.tx),
        };
    } else {
        network.rx = app_sum.rx;
        network.tx = app_sum.tx;
    }
    network.apps = net_apps
        .into_iter()
        .map(|(id, b)| {
            let info = apps.info(&id);
            NetApp { has_icon: apps.icon_path(&id).is_some(), name: info.name, id, rx: b.rx, tx: b.tx }
        })
        .collect();
    network.apps.sort_by_key(|x| Reverse(x.rx + x.tx));

    Ok(Report {
        from,
        to,
        bucket: if single { "hour" } else { "day" },
        buckets,
        total_ms,
        daily_avg_ms: total_ms / elapsed_days,
        compare_avg_ms,
        categories,
        apps: apps_out,
        sites,
        pickups,
        notifications,
        network,
    })
}

/// Splits [s, e) into pieces labeled with the focused app covering them (None = nothing
/// focused). `focused` is sorted and non-overlapping.
fn uncovered_by<'a>(focused: &[(i64, i64, &'a str)], s: i64, e: i64) -> Vec<(i64, i64, Option<&'a str>)> {
    let mut out = Vec::new();
    let mut cur = s;
    let first = focused.partition_point(|f| f.1 <= s);
    for &(fs, fe, id) in &focused[first..] {
        if fs >= e {
            break;
        }
        if fs > cur {
            out.push((cur, fs, None));
        }
        let (a, b) = (fs.max(cur), fe.min(e));
        if b > a {
            out.push((a, b, Some(id)));
            cur = b;
        }
    }
    if cur < e {
        out.push((cur, e, None));
    }
    out
}

/// Screen time in [from, to): the union of focused and background use.
fn screen_total(db: &Db, from: i64, to: i64) -> Result<i64> {
    let mut iv: Vec<(i64, i64)> = db
        .usage_between(from, to)?
        .into_iter()
        .filter(|r| r.kind != Kind::Web && !crate::is_ignored_app(&r.app_id))
        .map(|r| (r.start_ms.max(from), r.end_ms.min(to)))
        .collect();
    iv.sort_unstable();
    let (mut total, mut until) = (0, i64::MIN);
    for (s, e) in iv {
        let s = s.max(until);
        if e > s {
            total += e - s;
            until = e;
        }
    }
    Ok(total)
}

/// "3h 25m", "12m", "45s".
pub fn fmt_duration(ms: i64) -> String {
    let s = ms / 1000;
    let (h, m) = (s / 3600, (s % 3600) / 60);
    match (h, m) {
        (0, 0) => format!("{s}s"),
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

pub fn fmt_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if u == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(d: NaiveDate, h: i64, m: i64) -> i64 {
        day_start(d) + h * HOUR_MS + m * 60_000
    }

    #[test]
    fn splits_across_hours_and_reattributes_sites() {
        let db = Db::open_in_memory().unwrap();
        let d = NaiveDate::from_ymd_opt(2026, 10, 10).unwrap();
        // Code 9:30-10:30, Brave 11:00-11:40 of which 30m on youtube.
        let id = db.open_usage(at(d, 9, 30), "code.desktop", None).unwrap();
        db.extend_usage(id, at(d, 10, 30)).unwrap();
        let id = db.open_usage(at(d, 11, 0), "brave-browser.desktop", None).unwrap();
        db.extend_usage(id, at(d, 11, 40)).unwrap();
        let id = db.open_usage(at(d, 11, 5), "brave-browser.desktop", Some("www.youtube.com")).unwrap();
        db.extend_usage(id, at(d, 11, 35)).unwrap();
        db.add_pickup(at(d, 9, 0), "login", Some("code.desktop")).unwrap();
        db.add_notification(at(d, 9, 45), "slack.desktop").unwrap();

        let rules = Rules::builtin();
        let r = build(&db, &rules, &AppIndex::default(), d, d, &Filter::None, at(d, 23, 0)).unwrap();
        assert_eq!(r.buckets.len(), 24);
        assert_eq!(r.total_ms, 100 * 60_000);
        assert_eq!(r.buckets[9].total_ms, 30 * 60_000);
        assert_eq!(r.buckets[10].total_ms, 30 * 60_000);
        let cat = |n: &str| r.categories.iter().find(|c| c.name == n).map(|c| c.ms);
        assert_eq!(cat("Developer Tools"), Some(60 * 60_000));
        assert_eq!(cat("Entertainment"), Some(30 * 60_000));
        assert_eq!(r.sites[0].domain, "www.youtube.com");
        assert_eq!(r.pickups.count, 1);
        assert_eq!(r.pickups.first_apps[0].id, "code.desktop");
        assert_eq!(r.notifications.count, 1);

        let only = build(&db, &rules, &AppIndex::default(), d, d, &Filter::Category("Entertainment".into()), at(d, 23, 0)).unwrap();
        assert_eq!(only.total_ms, 30 * 60_000);
    }

    #[test]
    fn background_media_counts_without_double_counting() {
        let db = Db::open_in_memory().unwrap();
        let d = NaiveDate::from_ymd_opt(2026, 10, 10).unwrap();
        // VLC plays 10:00-10:36. Focus: VLC 10:00-10:05, Brave 10:05-10:30, VLC 10:30-10:36.
        let m = db.open_background(at(d, 10, 0), "vlc.desktop").unwrap();
        db.extend_usage(m, at(d, 10, 36)).unwrap();
        for (a, b, app) in [(0, 5, "vlc.desktop"), (5, 30, "brave-browser.desktop"), (30, 36, "vlc.desktop")] {
            let id = db.open_usage(at(d, 10, a), app, None).unwrap();
            db.extend_usage(id, at(d, 10, b)).unwrap();
        }
        // And 5 more minutes of VLC playing after the user walked away.
        let m = db.open_background(at(d, 11, 0), "vlc.desktop").unwrap();
        db.extend_usage(m, at(d, 11, 5)).unwrap();

        let r = build(&db, &Rules::builtin(), &AppIndex::default(), d, d, &Filter::None, at(d, 23, 0)).unwrap();
        let app = |id: &str| r.apps.iter().find(|a| a.id == id).unwrap().ms / 60_000;
        assert_eq!(app("vlc.desktop"), 41);
        assert_eq!(app("brave-browser.desktop"), 25);
        assert_eq!(r.total_ms / 60_000, 41);

        let vlc = build(&db, &Rules::builtin(), &AppIndex::default(), d, d, &Filter::App("vlc.desktop".into()), at(d, 23, 0)).unwrap();
        assert_eq!(vlc.total_ms / 60_000, 41);
    }

    #[test]
    fn week_has_daily_buckets() {
        let db = Db::open_in_memory().unwrap();
        let from = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 10, 11).unwrap();
        let r = build(&db, &Rules::builtin(), &AppIndex::default(), from, to, &Filter::None, at(to, 12, 0)).unwrap();
        assert_eq!(r.buckets.len(), 7);
        assert_eq!(r.bucket, "day");
    }

    #[test]
    fn formatting() {
        assert_eq!(fmt_duration(3_725_000), "1h 2m");
        assert_eq!(fmt_bytes(1_500_000), "1.5 MB");
    }
}

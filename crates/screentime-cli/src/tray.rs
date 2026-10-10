//! The top-right indicator: today's screen time at a glance, with a dropdown for the top
//! apps, pickups, notifications, data used, pause and the dashboard.
//!
//! Runs as its own process started with the desktop session and talks to screentimed over
//! loopback, so the daemon can restart without taking the icon down.

use std::fs::File;
use std::process::Command;
use std::time::Duration;

use anyhow::Result;
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{MenuItem, Tray, TrayMethods};
use screentime_core::report::{fmt_bytes, fmt_duration};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Status {
    pub tracking: bool,
    #[serde(default)]
    pub paused: bool,
    pub extension: bool,
    pub focus: Option<String>,
    #[serde(default)]
    pub idle: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct App {
    pub name: String,
    pub ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Count {
    pub count: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Net {
    pub rx: u64,
    pub tx: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Today {
    pub total_ms: i64,
    pub compare_avg_ms: Option<i64>,
    pub apps: Vec<App>,
    pub pickups: Count,
    pub notifications: Count,
    pub network: Net,
}

#[derive(Debug)]
struct ScreenTray {
    base: String,
    status: Option<Status>,
    today: Option<Today>,
}

fn open(url: String) {
    let _ = Command::new("xdg-open").arg(url).spawn();
}

fn info(label: String) -> MenuItem<ScreenTray> {
    StandardItem { label, enabled: false, ..Default::default() }.into()
}

impl Tray for ScreenTray {
    fn id(&self) -> String {
        "screentime".into()
    }

    fn title(&self) -> String {
        "Screen Time".into()
    }

    fn icon_name(&self) -> String {
        match &self.status {
            Some(s) if s.paused => "screentime-paused-symbolic".into(),
            _ => "screentime-symbolic".into(),
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let description = match (&self.status, &self.today) {
            (Some(_), Some(t)) => format!("Today {}", fmt_duration(t.total_ms)),
            _ => "screentimed is not running".into(),
        };
        ksni::ToolTip { title: "Screen Time".into(), description, ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        open(self.base.clone());
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let base = self.base.clone();
        let (Some(s), Some(t)) = (self.status.clone(), self.today.clone()) else {
            return vec![
                info("screentimed is not running".into()),
                MenuItem::Separator,
                StandardItem {
                    label: "Start it".into(),
                    activate: Box::new(|_: &mut Self| {
                        let _ = Command::new("systemctl").args(["--user", "start", "screentimed.service"]).status();
                    }),
                    ..Default::default()
                }
                .into(),
                StandardItem { label: "Quit".into(), activate: Box::new(|_: &mut Self| std::process::exit(0)), ..Default::default() }
                    .into(),
            ];
        };

        let mut items = vec![info(format!("Screen time today   {}", fmt_duration(t.total_ms)))];
        if let Some(avg) = t.compare_avg_ms.filter(|&a| a > 0) {
            let p = ((t.total_ms - avg) as f64 / avg as f64 * 100.0).round() as i64;
            let dir = if p >= 0 { "above" } else { "below" };
            items.push(info(format!("{}% {dir} your 7-day average", p.abs())));
        }
        if !s.extension && s.tracking {
            items.push(info("GNOME extension not active: apps are not tracked".into()));
        } else if let Some(f) = s.focus.as_ref().filter(|_| !s.paused && !s.idle) {
            items.push(info(format!("Now: {f}")));
        }

        items.push(MenuItem::Separator);
        items.push(info("Most used today".into()));
        for a in t.apps.iter().take(5) {
            let url = format!("{base}/#/day/{}", chrono::Local::now().format("%Y-%m-%d"));
            items.push(
                StandardItem {
                    label: format!("{}   {}", a.name, fmt_duration(a.ms)),
                    activate: Box::new(move |_: &mut Self| open(url.clone())),
                    ..Default::default()
                }
                .into(),
            );
        }
        if t.apps.is_empty() {
            items.push(info("Nothing yet".into()));
        }

        items.push(MenuItem::Separator);
        items.push(info(format!("Pickups {}   ·   Notifications {}", t.pickups.count, t.notifications.count)));
        items.push(info(format!(
            "Data used {}   (↓ {}  ↑ {})",
            fmt_bytes(t.network.rx + t.network.tx),
            fmt_bytes(t.network.rx),
            fmt_bytes(t.network.tx)
        )));

        items.push(MenuItem::Separator);
        let b = base.clone();
        items.push(
            StandardItem { label: "Open Screen Time".into(), activate: Box::new(move |_: &mut Self| open(b.clone())), ..Default::default() }
                .into(),
        );
        let b = base.clone();
        items.push(
            StandardItem {
                label: "This week".into(),
                activate: Box::new(move |_: &mut Self| open(format!("{b}/#/week/{}", chrono::Local::now().format("%Y-%m-%d")))),
                ..Default::default()
            }
            .into(),
        );
        if s.tracking {
            let b = base.clone();
            items.push(
                CheckmarkItem {
                    label: "Pause tracking".into(),
                    checked: s.paused,
                    activate: Box::new(move |t: &mut Self| {
                        let paused = !t.status.as_ref().is_some_and(|s| s.paused);
                        if let Some(s) = t.status.as_mut() {
                            s.paused = paused;
                        }
                        let url = format!("{b}/api/pause");
                        tokio::spawn(async move {
                            let _ = reqwest::Client::new()
                                .post(url)
                                .header("x-screentime", "1")
                                .json(&serde_json::json!({ "paused": paused }))
                                .send()
                                .await;
                        });
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }
        items.push(MenuItem::Separator);
        items.push(
            StandardItem { label: "Quit".into(), activate: Box::new(|_: &mut Self| std::process::exit(0)), ..Default::default() }
                .into(),
        );
        items
    }
}

/// Only one tray per session. The lock is released when the process exits.
fn claim_singleton() -> Option<File> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let f = File::create(dir.join("screentime-tray.lock")).ok()?;
    f.try_lock().ok()?;
    Some(f)
}

/// At login the AppIndicator extension may not own the StatusNotifierWatcher yet;
/// `assume_sni_available` makes ksni wait for it instead of failing.
async fn attach(base: &str) -> Result<ksni::Handle<ScreenTray>> {
    let mut wait = Duration::from_millis(250);
    let deadline = std::time::Instant::now() + Duration::from_secs(300);
    loop {
        let tray = ScreenTray { base: base.to_string(), status: None, today: None };
        match tray.assume_sni_available(true).spawn().await {
            Ok(h) => return Ok(h),
            Err(e) if std::time::Instant::now() >= deadline => return Err(e.into()),
            Err(_) => {}
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(Duration::from_secs(5));
    }
}

pub async fn run(base: String) -> Result<()> {
    let Some(_lock) = claim_singleton() else {
        // Already in the panel: treat the launch as "open the dashboard".
        open(base);
        return Ok(());
    };
    let mut handle = attach(&base).await?;
    let client = reqwest::Client::builder().timeout(Duration::from_secs(3)).build()?;
    loop {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let status: Option<Status> = get(&client, &format!("{base}/api/status")).await;
        let report: Option<Today> = get(&client, &format!("{base}/api/report?from={today}")).await;
        if handle.is_closed() {
            handle = attach(&base).await?;
        }
        handle
            .update(move |t: &mut ScreenTray| {
                t.status = status.clone();
                t.today = report.clone();
            })
            .await;
        tokio::time::sleep(Duration::from_secs(20)).await;
    }
}

async fn get<T: for<'de> Deserialize<'de>>(client: &reqwest::Client, url: &str) -> Option<T> {
    match client.get(url).send().await {
        Ok(r) if r.status().is_success() => r.json().await.ok(),
        _ => None,
    }
}

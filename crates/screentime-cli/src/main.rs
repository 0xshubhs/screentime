mod tray;

use anyhow::Result;
use chrono::{Datelike, Duration, Local, NaiveDate};
use screentime_core::apps::AppIndex;
use screentime_core::db::{self, Db};
use screentime_core::report::{self, fmt_bytes, fmt_duration, Filter, Report};
use screentime_core::Rules;

const USAGE: &str = "usage: screentime <command>

  today [DATE]            summary for a day (default today, DATE is YYYY-MM-DD)
  week [DATE]             summary for the Monday-to-Sunday week containing DATE
  open [day|week|month]   open the Screen Time app (or the dashboard in the browser)
  tray                    run the top bar indicator
  categorize <id|domain>  show which category an app id or domain falls in";

fn base_url() -> String {
    let listen = std::env::var("SCREENTIME_LISTEN").unwrap_or_else(|_| "127.0.0.1:47821".into());
    format!("http://{listen}")
}

/// Opens the native app when installed next to this binary or on PATH, otherwise the
/// dashboard in the default browser.
pub fn open_dashboard(base: &str, view: Option<&str>) {
    let sibling = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.join("screentime-app")));
    let app = sibling.filter(|p| p.is_file()).map(Into::into).unwrap_or_else(|| std::ffi::OsString::from("screentime-app"));
    let mut cmd = std::process::Command::new(app);
    cmd.args(view);
    if let Ok(mut child) = cmd.spawn() {
        // The tray is long-lived: reap the child when it exits so it does not linger as a
        // zombie (the app hands off to an already-open window and exits right away).
        std::thread::spawn(move || child.wait());
    } else {
        let hash = view.map(|v| format!("#/{v}/{}", Local::now().format("%Y-%m-%d"))).unwrap_or_default();
        let _ = std::process::Command::new("xdg-open").arg(format!("{base}/{hash}")).spawn();
    }
}

fn date_arg(arg: Option<String>) -> Result<NaiveDate> {
    Ok(match arg {
        Some(s) => NaiveDate::parse_from_str(&s, "%Y-%m-%d")?,
        None => Local::now().date_naive(),
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| "today".into());
    match cmd.as_str() {
        "today" => {
            let d = date_arg(args.next())?;
            print(&build(d, d)?, false);
        }
        "week" => {
            let d = date_arg(args.next())?;
            let from = d - Duration::days(d.weekday().num_days_from_monday() as i64);
            print(&build(from, from + Duration::days(6))?, true);
        }
        "open" => open_dashboard(&base_url(), args.next().as_deref()),
        "tray" => tray::run(base_url()).await?,
        "categorize" => {
            let target = args.next().unwrap_or_default();
            let rules = Rules::load();
            let category = if target.ends_with(".desktop") {
                rules.categorize_app(&target)
            } else {
                rules.categorize_domain(&target)
            };
            println!("{category}");
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
    Ok(())
}

fn build(from: NaiveDate, to: NaiveDate) -> Result<Report> {
    let path = std::env::var_os("SCREENTIME_DB").map(Into::into).unwrap_or_else(db::default_path);
    let db = Db::open_readonly(&path)?;
    let now = chrono::Utc::now().timestamp_millis();
    Ok(report::build(&db, &Rules::load(), &AppIndex::load(), from, to, &Filter::None, now)?)
}

fn print(r: &Report, multi: bool) {
    if multi {
        println!("{} to {}: {} total, {} daily average", r.from, r.to, fmt_duration(r.total_ms), fmt_duration(r.daily_avg_ms));
    } else {
        println!("{}: {} screen time", r.from, fmt_duration(r.total_ms));
    }
    if let Some(avg) = r.compare_avg_ms.filter(|&a| a > 0) {
        let cur = if multi { r.daily_avg_ms } else { r.total_ms };
        let p = ((cur - avg) as f64 / avg as f64 * 100.0).round() as i64;
        println!("  {}% {} {}", p.abs(), if p >= 0 { "above" } else { "below" }, if multi { "last week" } else { "the 7-day average" });
    }
    println!("\nCategories");
    for c in &r.categories {
        println!("  {:<26} {:>8}", c.name, fmt_duration(c.ms));
    }
    println!("\nMost used apps");
    for a in r.apps.iter().take(10) {
        println!("  {:<26} {:>8}  {}", a.name, fmt_duration(a.ms), a.category);
    }
    if !r.sites.is_empty() {
        println!("\nWebsites");
        for s in r.sites.iter().take(10) {
            println!("  {:<26} {:>8}  {}", s.domain, fmt_duration(s.ms), s.category);
        }
    }
    let p = &r.pickups;
    println!("\nPickups {} ({} logins, {} unlocks)", p.count, p.logins, p.unlocks);
    for a in p.first_apps.iter().take(5) {
        println!("  first used: {:<16} {}x", a.name, a.count);
    }
    println!("Notifications {}", r.notifications.count);
    for a in r.notifications.apps.iter().take(5) {
        println!("  {:<26} {}", a.name, a.count);
    }
    let n = &r.network;
    println!("Network {} (down {}, up {})", fmt_bytes(n.rx + n.tx), fmt_bytes(n.rx), fmt_bytes(n.tx));
    for a in n.apps.iter().take(5) {
        println!("  {:<26} {}", a.name, fmt_bytes(a.rx + a.tx));
    }
}

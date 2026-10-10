//! screentimed: tracks app, site, pickup, notification and network usage and serves the
//! dashboard on http://127.0.0.1:47821. See docs/PLAN.md.

mod http;
mod net;
mod notify;
mod session;
mod tracker;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use screentime_core::apps::AppIndex;
use screentime_core::db::{self, Db};
use screentime_core::Rules;
use tokio::signal::unix::{signal, SignalKind};

use tracker::Tracker;

pub const DEFAULT_LISTEN: &str = "127.0.0.1:47821";

pub struct Shared {
    state: Mutex<(Db, Tracker)>,
    rules: Rules,
    pub apps: RwLock<AppIndex>,
    pub tracking: bool,
    pub extension_ok: AtomicBool,
    pub web_seen_ms: AtomicI64,
}

impl Shared {
    /// Runs `f` with the database and tracker locked, as one transaction: every write an
    /// event makes (close one row, open the next) commits together or not at all. Errors
    /// are logged, not propagated.
    pub fn with<T>(&self, f: impl FnOnce(&Db, &mut Tracker, &Rules) -> db::Result<T>) -> Option<T> {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let (db, tracker) = &mut *guard;
        db.atomically(|db| f(db, tracker, &self.rules)).map_err(|e| eprintln!("db: {e}")).ok()
    }
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

struct Args {
    serve_only: bool,
    listen: String,
    db: PathBuf,
}

fn args() -> Args {
    let mut a = Args {
        serve_only: false,
        listen: std::env::var("SCREENTIME_LISTEN").unwrap_or_else(|_| DEFAULT_LISTEN.into()),
        db: std::env::var_os("SCREENTIME_DB").map(PathBuf::from).unwrap_or_else(db::default_path),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--serve-only" => a.serve_only = true,
            "--listen" => a.listen = it.next().expect("--listen ADDR"),
            "--db" => a.db = it.next().expect("--db PATH").into(),
            "--version" => {
                println!("screentimed {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            _ => {
                eprintln!("usage: screentimed [--serve-only] [--listen ADDR] [--db PATH]");
                std::process::exit(2);
            }
        }
    }
    a
}

/// App tracking needs the GNOME Shell extension. The .deb installs it system-wide, which GNOME
/// does not enable on its own, so turn it on for this user if it is off. If the shell has not
/// seen it yet (installed during this session), it is added to the list for the next login.
fn ensure_extension_enabled() {
    use std::process::Command;
    const UUID: &str = "screentime@0xshubhs";
    if !std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().contains("GNOME") {
        return;
    }
    let Ok(out) = Command::new("gsettings").args(["get", "org.gnome.shell", "enabled-extensions"]).output() else {
        return;
    };
    let current = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || current.contains(UUID) {
        return;
    }
    if Command::new("gnome-extensions").args(["enable", UUID]).status().is_ok_and(|s| s.success()) {
        return;
    }
    let list = if current == "@as []" || current == "[]" {
        format!("['{UUID}']")
    } else {
        format!("{}, '{UUID}']", current.trim_end_matches(']'))
    };
    let _ = Command::new("gsettings").args(["set", "org.gnome.shell", "enabled-extensions", &list]).status();
}

/// Runs a background task, logging why it stopped.
fn spawn<F>(name: &'static str, fut: F)
where
    F: std::future::Future<Output = zbus::Result<()>> + Send + 'static,
{
    tokio::spawn(async move {
        if let Err(e) = fut.await {
            eprintln!("{name}: {e}");
        }
    });
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = args();
    let db = if args.serve_only { Db::open_readonly(&args.db)? } else { Db::open(&args.db)? };
    let shared = Arc::new(Shared {
        state: Mutex::new((db, Tracker::default())),
        rules: Rules::load(),
        apps: RwLock::new(AppIndex::load()),
        tracking: !args.serve_only,
        extension_ok: AtomicBool::new(false),
        web_seen_ms: AtomicI64::new(0),
    });

    if shared.tracking {
        ensure_extension_enabled();
        match zbus::Connection::session().await {
            Ok(session) => {
                spawn("focus", session::focus(shared.clone(), session.clone()));
                spawn("idle", session::idle(shared.clone(), session.clone()));
                spawn("media", session::media(shared.clone(), session.clone()));
                spawn("on_screen", session::on_screen(shared.clone(), session.clone()));
                spawn("screensaver", session::screensaver(shared.clone(), session));
                spawn("notifications", notify::monitor(shared.clone()));
            }
            Err(e) => eprintln!("session bus unavailable, app tracking disabled: {e}"),
        }
        match zbus::Connection::system().await {
            Ok(system) => {
                match session::login_time(&system).await {
                    Ok(ts) => {
                        shared.with(|db, t, _| t.on_login(db, ts));
                    }
                    Err(e) => eprintln!("login time: {e}"),
                }
                spawn("sleep", session::sleep(shared.clone(), system));
            }
            Err(e) => eprintln!("system bus unavailable: {e}"),
        }
        tokio::spawn(net::sampler(shared.clone()));

        let hb = shared.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(30));
            loop {
                tick.tick().await;
                hb.with(|db, t, _| t.heartbeat(db, now_ms()));
            }
        });
    }

    // Pick up newly installed apps.
    let idx = shared.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(600)).await;
            let fresh = tokio::task::spawn_blocking(AppIndex::load).await.unwrap_or_default();
            *idx.apps.write().unwrap() = fresh;
        }
    });

    let listener = tokio::net::TcpListener::bind(&args.listen).await?;
    eprintln!(
        "screentimed {} {} on http://{} (db {})",
        env!("CARGO_PKG_VERSION"),
        if shared.tracking { "tracking, dashboard" } else { "dashboard only" },
        args.listen,
        args.db.display()
    );
    let server = axum::serve(listener, http::router(shared.clone()));

    let mut term = signal(SignalKind::terminate())?;
    tokio::select! {
        r = server => r?,
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    shared.with(|db, t, _| t.shutdown(db, now_ms()));
    eprintln!("screentimed stopped");
    Ok(())
}

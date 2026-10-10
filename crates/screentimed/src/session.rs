//! Desktop session inputs over D-Bus: focused app (our GNOME Shell extension), idle time
//! (Mutter), lock state (GNOME ScreenSaver), suspend and login time (logind).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;
use zbus::{Connection, Proxy};

use crate::{now_ms, Shared};

async fn proxy<'a>(conn: &Connection, dest: &'a str, path: &'a str, iface: &'a str) -> zbus::Result<Proxy<'a>> {
    zbus::proxy::Builder::new(conn)
        .destination(dest)?
        .path(path)?
        .interface(iface)?
        .cache_properties(CacheProperties::No)
        .build()
        .await
}

/// Focus changes pushed by the extension, plus a slow poll that recovers if the extension
/// is enabled after the daemon starts (or gnome-shell restarts).
pub async fn focus(shared: Arc<Shared>, conn: Connection) -> zbus::Result<()> {
    let shell = proxy(&conn, "org.gnome.Shell", "/dev/zeroxshubhs/Screentime", "dev.zeroxshubhs.Screentime.Shell").await?;
    let mut changes = shell.receive_signal("FocusChanged").await?;
    let mut poll = tokio::time::interval(Duration::from_secs(10));
    loop {
        let focused = tokio::select! {
            Some(msg) = changes.next() => msg.body().deserialize::<(String, String)>().ok(),
            _ = poll.tick() => match shell.call::<_, _, (String, String)>("GetFocused", &()).await {
                Ok(v) => Some(v),
                Err(e) => {
                    if shared.extension_ok.swap(false, Ordering::Relaxed) {
                        eprintln!("focus: GNOME extension not reachable ({e}); is screentime@0xshubhs enabled?");
                    }
                    None
                }
            },
        };
        if let Some((app, _title)) = focused {
            shared.extension_ok.store(true, Ordering::Relaxed);
            shared.with(|db, t, _| t.on_focus(db, now_ms(), Some(app)));
        }
    }
}

pub async fn idle(shared: Arc<Shared>, conn: Connection) -> zbus::Result<()> {
    let mon = proxy(&conn, "org.gnome.Mutter.IdleMonitor", "/org/gnome/Mutter/IdleMonitor/Core", "org.gnome.Mutter.IdleMonitor").await?;
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    loop {
        tick.tick().await;
        match mon.call::<_, _, u64>("GetIdletime", &()).await {
            Ok(ms) => {
                shared.with(|db, t, rules| t.on_idle_time(db, rules, now_ms(), ms as i64));
            }
            Err(e) => eprintln!("idle: {e}"),
        }
    }
}

/// Lock and unlock. Each unlock is a pickup.
pub async fn screensaver(shared: Arc<Shared>, conn: Connection) -> zbus::Result<()> {
    let ss = proxy(&conn, "org.gnome.ScreenSaver", "/org/gnome/ScreenSaver", "org.gnome.ScreenSaver").await?;
    let mut changes = ss.receive_signal("ActiveChanged").await?;
    if let Ok(active) = ss.call::<_, _, bool>("GetActive", &()).await {
        shared.with(|db, t, _| t.on_lock(db, now_ms(), active));
    }
    while let Some(msg) = changes.next().await {
        if let Ok(active) = msg.body().deserialize::<bool>() {
            shared.with(|db, t, _| t.on_lock(db, now_ms(), active));
        }
    }
    Ok(())
}

pub async fn sleep(shared: Arc<Shared>, system: Connection) -> zbus::Result<()> {
    let mgr = proxy(&system, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager").await?;
    let mut changes = mgr.receive_signal("PrepareForSleep").await?;
    while let Some(msg) = changes.next().await {
        if let Ok(sleeping) = msg.body().deserialize::<bool>() {
            shared.with(|db, t, _| t.on_sleep(db, now_ms(), sleeping));
        }
    }
    Ok(())
}

/// Start time of the user's graphical session, in Unix ms.
pub async fn login_time(system: &Connection) -> zbus::Result<i64> {
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(1000);
    let mgr = proxy(system, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager").await?;
    let user: OwnedObjectPath = mgr.call("GetUser", &(uid,)).await?;
    let user = proxy(system, "org.freedesktop.login1", user.as_str(), "org.freedesktop.login1.User").await?;
    let (_, session): (String, OwnedObjectPath) = user.get_property("Display").await?;
    let session = proxy(system, "org.freedesktop.login1", session.as_str(), "org.freedesktop.login1.Session").await?;
    let usec: u64 = session.get_property("Timestamp").await?;
    Ok((usec / 1000) as i64)
}

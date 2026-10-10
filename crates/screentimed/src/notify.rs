//! Counts notifications per app by eavesdropping (as a D-Bus monitor) on the two ways apps
//! post them: `org.freedesktop.Notifications.Notify` and `org.gtk.Notifications.AddNotification`
//! (which also carries notifications forwarded by the portal for Flatpak apps).

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::StreamExt;
use zbus::message::Type;
use zbus::zvariant::OwnedValue;
use zbus::{Connection, MatchRule, MessageStream};

use crate::{now_ms, Shared};

/// Re-posting the same GTK notification id within this window is an update, not a new one.
const DEDUPE_MS: i64 = 60_000;

pub async fn monitor(shared: Arc<Shared>) -> zbus::Result<()> {
    let conn = Connection::session().await?;
    let rules = [
        MatchRule::builder().msg_type(Type::MethodCall).interface("org.freedesktop.Notifications")?.member("Notify")?.build(),
        MatchRule::builder().msg_type(Type::MethodCall).interface("org.gtk.Notifications")?.member("AddNotification")?.build(),
    ];
    zbus::fdo::MonitoringProxy::new(&conn).await?.become_monitor(&rules, 0).await?;

    let mut recent: HashMap<(String, String), i64> = HashMap::new();
    let mut stream = MessageStream::from(&conn);
    while let Some(msg) = stream.next().await {
        let Ok(msg) = msg else { continue };
        let header = msg.header();
        let now = now_ms();
        let app = match header.member().map(|m| m.as_str()) {
            Some("Notify") => {
                type Notify = (String, u32, String, String, String, Vec<String>, HashMap<String, OwnedValue>, i32);
                let Ok((app_name, replaces_id, ..,  hints, _)) = msg.body().deserialize::<Notify>() else { continue };
                // GNOME relays each call to the shell with x-shell-* hints added; count the
                // original only.
                if replaces_id != 0 || hints.contains_key("x-shell-sender") {
                    continue;
                }
                let entry = hints.get("desktop-entry").and_then(|v| String::try_from(v.clone()).ok());
                let pid = hints.get("sender-pid").and_then(|v| i64::try_from(v.clone()).ok());
                match (entry.filter(|e| !e.is_empty()), pid) {
                    (Some(e), _) => format!("{}.desktop", e.trim_end_matches(".desktop")),
                    (None, Some(pid)) => app_for_pid(&shared, pid as u32).unwrap_or_else(|| shared.apps.read().unwrap().id_for_name(&app_name)),
                    (None, None) if app_name.is_empty() => continue,
                    (None, None) => shared.apps.read().unwrap().id_for_name(&app_name),
                }
            }
            Some("AddNotification") => {
                let Ok((app_id, id, _)) = msg.body().deserialize::<(String, String, HashMap<String, OwnedValue>)>() else { continue };
                recent.retain(|_, ts| now - *ts < DEDUPE_MS);
                if recent.insert((app_id.clone(), id), now).is_some() {
                    continue;
                }
                format!("{app_id}.desktop")
            }
            _ => continue,
        };
        shared.with(|db, _, _| db.add_notification(now, &app));
    }
    Ok(())
}

fn app_for_pid(shared: &Shared, pid: u32) -> Option<String> {
    let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    let cgroup = cgroup.lines().find_map(|l| l.strip_prefix("0::"))?.to_string();
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?.trim().to_string();
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok();
    Some(crate::net::app_for(shared, Some(&cgroup), Some(&comm), exe.as_deref()))
}

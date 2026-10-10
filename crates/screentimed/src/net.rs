//! Network data. Totals come from physical interface counters (/proc/net/dev). Per-app
//! figures are sampled from TCP socket stats (`ss`), which needs no root but misses UDP/QUIC
//! and connections that open and close between samples; the report shows that remainder
//! as unattributed.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use screentime_core::apps::id_from_cgroup;

use crate::{now_ms, Shared};

const INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug, PartialEq)]
pub struct Sock {
    pub ino: u64,
    pub comm: Option<String>,
    pub pid: Option<u32>,
    pub cgroup: Option<String>,
    pub rx: u64,
    pub tx: u64,
}

/// Parses one line of `ss -tinpeHO`.
pub fn parse_ss_line(line: &str) -> Option<Sock> {
    let mut cols = line.split_whitespace();
    let peer = cols.nth(4)?;
    if is_loopback(peer) {
        return None;
    }
    let field = |key: &str| -> Option<&str> {
        line.split_whitespace().find_map(|t| t.strip_prefix(key))
    };
    let num = |key: &str| field(key).and_then(|v| v.parse::<u64>().ok());
    let (comm, pid) = match line.find("users:((\"") {
        Some(i) => {
            let rest = &line[i + 9..];
            let end = rest.find('"')?;
            let pid = rest[end..].split("pid=").nth(1).and_then(|p| p.split([',', ')']).next()?.parse().ok());
            (Some(rest[..end].to_string()), pid)
        }
        None => (None, None),
    };
    Some(Sock {
        ino: num("ino:")?,
        comm,
        pid,
        cgroup: field("cgroup:").map(str::to_string),
        rx: num("bytes_received:").unwrap_or(0),
        tx: num("bytes_acked:").unwrap_or(0),
    })
}

fn is_loopback(addr: &str) -> bool {
    addr.starts_with("127.") || addr.starts_with("[::1]") || addr.starts_with("[::ffff:127.")
}

/// Physical interfaces only (virtual ones like docker0, veth, tailscale0 would double count).
fn physical_ifaces() -> HashMap<String, (u64, u64)> {
    let Ok(text) = std::fs::read_to_string("/proc/net/dev") else { return HashMap::new() };
    text.lines()
        .skip(2)
        .filter_map(|l| {
            let (name, rest) = l.split_once(':')?;
            let name = name.trim();
            if !Path::new("/sys/class/net").join(name).join("device").exists() {
                return None;
            }
            let v: Vec<u64> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            Some((name.to_string(), (*v.first()?, *v.get(8)?)))
        })
        .collect()
}

fn iface_kind(name: &str) -> &'static str {
    let p = Path::new("/sys/class/net").join(name);
    if p.join("wireless").exists() || p.join("phy80211").exists() {
        "wifi"
    } else {
        "ethernet"
    }
}

pub async fn sampler(shared: Arc<Shared>) {
    let mut last_socks: Option<HashMap<u64, (u64, u64)>> = None;
    let mut last_ifaces: Option<HashMap<String, (u64, u64)>> = None;
    let mut app_cache: HashMap<(String, Option<u32>), String> = HashMap::new();
    let mut tick = tokio::time::interval(INTERVAL);
    loop {
        tick.tick().await;
        let now = now_ms();

        let ifaces = physical_ifaces();
        if let Some(prev) = &last_ifaces {
            let deltas: Vec<_> = ifaces
                .iter()
                .filter_map(|(name, &(rx, tx))| {
                    let &(prx, ptx) = prev.get(name)?;
                    // Counters reset when an interface goes down and up again.
                    let d = |c: u64, p: u64| if c >= p { c - p } else { c };
                    Some((name.clone(), d(rx, prx), d(tx, ptx)))
                })
                .filter(|(_, rx, tx)| rx + tx > 0)
                .collect();
            shared.with(|db, _, _| {
                for (name, rx, tx) in &deltas {
                    db.add_net_iface(now, name, iface_kind(name), *rx, *tx)?;
                }
                Ok(())
            });
        }
        last_ifaces = Some(ifaces);

        let out = match tokio::process::Command::new("ss").arg("-tinpeHO").output().await {
            Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
            Err(e) => {
                eprintln!("net: running ss failed: {e}");
                continue;
            }
        };
        let socks: Vec<Sock> = out.lines().filter_map(parse_ss_line).collect();
        let mut per_app: HashMap<String, (u64, u64)> = HashMap::new();
        if let Some(prev) = &last_socks {
            for s in &socks {
                let (prx, ptx) = prev.get(&s.ino).copied().unwrap_or((0, 0));
                let (drx, dtx) = (s.rx.saturating_sub(prx), s.tx.saturating_sub(ptx));
                if drx + dtx == 0 {
                    continue;
                }
                let app = cached_app_for(&shared, &mut app_cache, s);
                let e = per_app.entry(app).or_default();
                e.0 += drx;
                e.1 += dtx;
            }
        }
        last_socks = Some(socks.iter().map(|s| (s.ino, (s.rx, s.tx))).collect());
        if app_cache.len() > 4096 {
            app_cache.clear();
        }
        if !per_app.is_empty() {
            shared.with(|db, _, _| {
                for (app, (rx, tx)) in &per_app {
                    db.add_net_app(now, app, *rx, *tx)?;
                }
                Ok(())
            });
        }
    }
}

/// App key for a process: desktop id from its systemd scope when present, otherwise matched
/// by executable, otherwise `proc:<name>` (or `sys:<unit>` for other users' services).
pub fn app_for(shared: &Shared, cgroup: Option<&str>, comm: Option<&str>, exe: Option<&Path>) -> String {
    if let Some(id) = cgroup.and_then(id_from_cgroup) {
        return id;
    }
    let unit = cgroup.and_then(|c| c.rsplit('/').next()).unwrap_or("");
    let Some(comm) = comm else {
        return match unit.strip_suffix(".service") {
            Some(name) => format!("sys:{name}"),
            None => "sys:system".into(),
        };
    };
    // Commands run in a terminal tab are CLI tools, not the terminal app itself.
    if unit.starts_with("vte-spawn-") || unit.starts_with("ptyxis-spawn-") || unit.contains("terminal") {
        return format!("proc:{comm}");
    }
    shared.apps.read().unwrap().id_for_process(comm, exe)
}

fn cached_app_for(shared: &Shared, cache: &mut HashMap<(String, Option<u32>), String>, s: &Sock) -> String {
    let key = (format!("{}|{}", s.comm.as_deref().unwrap_or(""), s.cgroup.as_deref().unwrap_or("")), s.pid);
    cache
        .entry(key)
        .or_insert_with(|| {
            let exe = s.pid.and_then(|p| std::fs::read_link(format!("/proc/{p}/exe")).ok());
            app_for(shared, s.cgroup.as_deref(), s.comm.as_deref(), exe.as_deref())
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ss_oneline() {
        let l = r#"ESTAB      0      0                 192.168.0.113:35994   160.79.104.10:443   users:(("claude",pid=10836,fd=12)) timer:(keepalive,59sec,0) uid:1000 ino:70882 sk:1 cgroup:/user.slice/user-1000.slice/user@1000.service/app.slice/ptyxis-spawn-550f.scope <-> ts sack cubic bytes_sent:100860 bytes_acked:100861 bytes_received:170067 segs_out:1833"#;
        let s = parse_ss_line(l).unwrap();
        assert_eq!(s.ino, 70882);
        assert_eq!(s.comm.as_deref(), Some("claude"));
        assert_eq!(s.pid, Some(10836));
        assert_eq!((s.rx, s.tx), (170067, 100861));
        let lo = "ESTAB 0 0 127.0.0.1:5000 127.0.0.1:6000 ino:5";
        assert_eq!(parse_ss_line(lo), None);
    }
}

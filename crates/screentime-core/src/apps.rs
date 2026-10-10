//! Resolves app ids (desktop file ids, `wm:<class>`, `proc:<name>`) to display names and icons
//! by indexing installed `.desktop` files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::db::home;

#[derive(Debug, Clone, Default)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    exec: Option<String>,
    wm_class: Option<String>,
    hidden: bool,
}

#[derive(Debug, Default)]
pub struct AppIndex {
    by_id: HashMap<String, AppInfo>,
    icon_dirs: Vec<PathBuf>,
}

impl AppIndex {
    /// Scans the XDG data dirs (plus snap and flatpak export dirs) for desktop files.
    pub fn load() -> Self {
        let mut idx = AppIndex::default();
        let data_dirs = data_dirs();
        // Earlier dirs win, so walk in reverse and let later inserts override.
        for dir in data_dirs.iter().rev() {
            let Ok(entries) = std::fs::read_dir(dir.join("applications")) else { continue };
            for e in entries.flatten() {
                let path = e.path();
                let Some(file) = path.file_name().and_then(|f| f.to_str()) else { continue };
                if !file.ends_with(".desktop") {
                    continue;
                }
                if let Some(info) = parse_desktop(&path, file) {
                    idx.by_id.insert(file.to_string(), info);
                }
            }
        }
        for dir in &data_dirs {
            for theme in ["hicolor", "Yaru", "Adwaita"] {
                for size in ["256x256", "128x128", "scalable", "96x96", "64x64", "48x48", "512x512"] {
                    idx.icon_dirs.push(dir.join("icons").join(theme).join(size).join("apps"));
                }
            }
            idx.icon_dirs.push(dir.join("pixmaps"));
        }
        idx.icon_dirs.retain(|d| d.is_dir());
        idx
    }

    /// Display name and icon file for any app key the daemon stores.
    pub fn info(&self, id: &str) -> AppInfo {
        if let Some(info) = self.lookup(id) {
            return info.clone();
        }
        let name = id
            .strip_prefix("proc:")
            .or_else(|| id.strip_prefix("wm:"))
            .or_else(|| id.strip_prefix("name:"))
            .or_else(|| id.strip_prefix("sys:"))
            .unwrap_or_else(|| id.strip_suffix(".desktop").unwrap_or(id));
        AppInfo { id: id.to_string(), name: pretty(name), ..Default::default() }
    }

    fn lookup(&self, id: &str) -> Option<&AppInfo> {
        if let Some(i) = self.by_id.get(id) {
            return Some(i);
        }
        if let Some(class) = id.strip_prefix("wm:") {
            let lc = class.to_ascii_lowercase();
            return self.by_id.values().find(|i| {
                i.wm_class.as_deref().map(str::to_ascii_lowercase).as_deref() == Some(lc.as_str())
                    || stem(&i.id).to_ascii_lowercase() == lc
            });
        }
        if let Some(name) = id.strip_prefix("name:") {
            return self.by_id.values().find(|i| i.name.eq_ignore_ascii_case(name));
        }
        None
    }

    /// Canonical id for an app named in a notification (`desktop-entry` hint or app name).
    pub fn id_for_name(&self, name: &str) -> String {
        match self.lookup(&format!("name:{name}")) {
            Some(i) => i.id.clone(),
            None => format!("name:{name}"),
        }
    }

    /// Desktop id for an MPRIS player from its DesktopEntry, Identity and bus name suffix
    /// (`org.mpris.MediaPlayer2.<suffix>`, e.g. "vlc" or "brave.instance123").
    pub fn id_for_player(&self, entry: &str, identity: &str, bus_suffix: &str) -> String {
        let entry = entry.trim_end_matches(".desktop");
        if !entry.is_empty() {
            let id = format!("{entry}.desktop");
            if self.by_id.contains_key(&id) {
                return id;
            }
        }
        if !identity.is_empty() {
            if let Some(i) = self.lookup(&format!("name:{identity}")) {
                return i.id.clone();
            }
        }
        let key = if entry.is_empty() { bus_suffix.split('.').next().unwrap_or("") } else { entry }.to_ascii_lowercase();
        if key.len() >= 3 {
            let found = self
                .by_id
                .values()
                .filter(|i| !i.hidden && stem(&i.id).to_ascii_lowercase().contains(&key))
                .min_by_key(|i| i.id.len());
            if let Some(i) = found {
                return i.id.clone();
            }
        }
        if !entry.is_empty() {
            format!("{entry}.desktop")
        } else {
            format!("name:{}", if identity.is_empty() { bus_suffix } else { identity })
        }
    }

    /// Best desktop id for a process, given its command name and executable path.
    pub fn id_for_process(&self, comm: &str, exe: Option<&Path>) -> String {
        let exe_real = exe.and_then(|p| std::fs::canonicalize(p).ok());
        let comm_lc = comm.to_ascii_lowercase();
        let mut best: Option<(&AppInfo, u8)> = None;
        for info in self.by_id.values().filter(|i| !i.hidden) {
            let Some(exec) = info.exec.as_deref() else { continue };
            let exec_path = Path::new(exec);
            let score = if exe_real.is_some()
                && exec_path.is_absolute()
                && std::fs::canonicalize(exec_path).ok() == exe_real
            {
                4
            } else if exec_path.file_name().and_then(|f| f.to_str()) == Some(comm) {
                3
            } else if info.wm_class.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(comm)) {
                2
            } else if comm_lc.len() >= 4 && stem(&info.id).to_ascii_lowercase().contains(&comm_lc) {
                1
            } else {
                0
            };
            // Prefer native packages over the flatpak/snap duplicates on ties.
            if score > 0 && best.is_none_or(|(b, s)| score > s || (score == s && info.id.len() < b.id.len())) {
                best = Some((info, score));
            }
        }
        best.map_or_else(|| format!("proc:{comm}"), |(i, _)| i.id.clone())
    }

    /// Absolute path of an icon file for the app, if one can be found.
    pub fn icon_path(&self, id: &str) -> Option<PathBuf> {
        let icon = self.lookup(id)?.icon.as_deref()?;
        let p = Path::new(icon);
        if p.is_absolute() {
            return p.is_file().then(|| p.to_path_buf());
        }
        for dir in &self.icon_dirs {
            for ext in ["png", "svg", "xpm"] {
                let f = dir.join(format!("{icon}.{ext}"));
                if f.is_file() {
                    return Some(f);
                }
            }
        }
        None
    }
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"))];
    let sys = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(sys.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    for extra in [
        home().join(".local/share/flatpak/exports/share"),
        "/var/lib/flatpak/exports/share".into(),
        "/var/lib/snapd/desktop".into(),
        "/usr/share".into(),
    ] {
        if !dirs.contains(&extra) {
            dirs.push(extra);
        }
    }
    dirs
}

fn parse_desktop(path: &Path, file: &str) -> Option<AppInfo> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut info = AppInfo { id: file.to_string(), ..Default::default() };
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            if in_entry {
                break;
            }
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "Name" => info.name = v.trim().to_string(),
            "Icon" => info.icon = Some(v.trim().to_string()),
            "NoDisplay" => info.hidden = v.trim() == "true",
            "StartupWMClass" => info.wm_class = Some(v.trim().to_string()),
            "Exec" => info.exec = v.split_whitespace().find(|t| !t.contains('=') && *t != "env").map(str::to_string),
            _ => {}
        }
    }
    (!info.name.is_empty()).then_some(info)
}

fn stem(id: &str) -> &str {
    id.strip_suffix(".desktop").unwrap_or(id)
}

fn pretty(s: &str) -> String {
    // org.telegram.desktop -> Telegram
    let last = s.rsplit('.').find(|p| !p.eq_ignore_ascii_case("desktop") && !p.is_empty()).unwrap_or(s);
    let mut c = last.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => s.to_string(),
    }
}

/// Maps a systemd cgroup path to a desktop id when the scope encodes one.
pub fn id_from_cgroup(cgroup: &str) -> Option<String> {
    let unit = cgroup.rsplit('/').next()?;
    if let Some(rest) = unit.strip_prefix("app-flatpak-").and_then(|r| r.strip_suffix(".scope")) {
        let (id, _) = rest.rsplit_once('-')?;
        return Some(format!("{}.desktop", unescape(id)));
    }
    if let Some(rest) = unit.strip_prefix("app-gnome-").and_then(|r| r.strip_suffix(".scope")) {
        let (id, _) = rest.rsplit_once('-')?;
        return Some(format!("{}.desktop", unescape(id)));
    }
    if let Some(rest) = unit.strip_prefix("snap.").and_then(|r| r.strip_suffix(".scope")) {
        let mut parts = rest.splitn(3, '.');
        let snap = parts.next()?;
        // The scope suffix is "-<uuid>" (36 chars); app names may themselves contain dashes.
        let rest = parts.next()?;
        let app = match rest.len().checked_sub(37) {
            Some(i) if rest.as_bytes()[i] == b'-' => &rest[..i],
            _ => rest.split_once('-').map_or(rest, |(a, _)| a),
        };
        let app = if app.is_empty() { snap } else { app };
        return Some(format!("{snap}_{app}.desktop"));
    }
    None
}

/// Undoes systemd unit name escaping (`\x2d` -> `-`).
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1] == b'x' {
            if let Ok(v) = u8::from_str_radix(&s[i + 2..i + 4], 16) {
                out.push(v as char);
                i += 4;
                continue;
            }
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cgroups() {
        let base = "/user.slice/user-1000.slice/user@1000.service/app.slice/";
        assert_eq!(
            id_from_cgroup(&format!("{base}app-gnome-google\\x2dchrome-4242.scope")).as_deref(),
            Some("google-chrome.desktop")
        );
        assert_eq!(
            id_from_cgroup(&format!("{base}app-flatpak-com.brave.Browser-12.scope")).as_deref(),
            Some("com.brave.Browser.desktop")
        );
        assert_eq!(
            id_from_cgroup(&format!("{base}snap.firefox.firefox-550ffb21-c5e0-4541-be1b-d81b9c2ae0af.scope")).as_deref(),
            Some("firefox_firefox.desktop")
        );
        assert_eq!(id_from_cgroup(&format!("{base}ptyxis-spawn-550ffb21.scope")), None);
    }

    #[test]
    fn fallback_names() {
        let idx = AppIndex::default();
        assert_eq!(idx.info("proc:cargo").name, "Cargo");
        assert_eq!(idx.info("org.example.Thing.desktop").name, "Thing");
    }
}

//! The usage state machine. Inputs (focus, idle, lock, sleep, browser tab) come in as events;
//! the tracker keeps at most one open `app` row and one open `web` row and closes or opens
//! them whenever the "what is the user doing right now" answer changes.

use std::collections::{HashMap, HashSet};

use screentime_core::db::{Db, Result};
#[cfg(test)]
use screentime_core::db::Kind;
use screentime_core::Rules;

/// Idle time after which usage stops counting (unless media is playing).
pub const IDLE_MS: i64 = 2 * 60 * 1000;
/// After an unlock, focus changes within this window replace the pickup's first app.
const PICKUP_GRACE_MS: i64 = 5_000;

/// Substrings of desktop ids that identify browsers, paired with the family name the
/// browser extension reports.
const BROWSERS: &[(&str, &str)] = &[
    ("brave", "brave"),
    ("chrome", "chrome"),
    ("chromium", "chrome"),
    ("firefox", "firefox"),
    ("librewolf", "firefox"),
    ("zen", "firefox"),
    ("edge", "edge"),
    ("vivaldi", "vivaldi"),
    ("opera", "opera"),
];

#[derive(Debug, Clone, Default)]
pub struct WebState {
    pub domain: Option<String>,
    pub audible: bool,
}

#[derive(Debug)]
struct Open {
    row: i64,
    app: String,
    domain: Option<String>,
}

#[derive(Debug)]
struct PendingPickup {
    id: i64,
    deadline: Option<i64>,
}

#[derive(Debug, Default)]
pub struct Tracker {
    pub focus: Option<String>,
    pub idle: bool,
    pub locked: bool,
    pub sleeping: bool,
    /// Set from the tray or dashboard; nothing is recorded while paused.
    pub paused: bool,
    /// Apps currently playing media (MPRIS PlaybackStatus = Playing).
    pub playing: HashSet<String>,
    /// Open `media` rows by app.
    open_media: HashMap<String, i64>,
    web: HashMap<String, WebState>,
    open_app: Option<Open>,
    open_web: Option<Open>,
    pickup: Option<PendingPickup>,
}

pub fn browser_family(app_id: &str) -> Option<&'static str> {
    let id = app_id.to_ascii_lowercase();
    BROWSERS.iter().find(|(k, _)| id.contains(k)).map(|(_, f)| *f)
}

impl Tracker {
    pub fn is_active(&self) -> bool {
        !self.idle && !self.locked && !self.sleeping && !self.paused && self.focus.is_some()
    }

    fn current_web(&self) -> Option<&WebState> {
        let family = browser_family(self.focus.as_deref()?)?;
        self.web.get(family).or_else(|| (self.web.len() == 1).then(|| self.web.values().next()).flatten())
    }

    pub fn current_domain(&self) -> Option<&str> {
        self.current_web()?.domain.as_deref()
    }

    /// Media keeps the clock running while the user is not touching the keyboard.
    /// The focused app is playing audio/video, so no input does not mean the user left.
    fn media_playing(&self) -> bool {
        self.current_web().is_some_and(|w| w.audible) || self.focus.as_ref().is_some_and(|f| self.playing.contains(f))
    }

    /// Playback counts while the screen is on (not locked, asleep or paused), focused or not.
    fn sync_media(&mut self, db: &Db, now: i64) -> Result<()> {
        let screen_on = !self.locked && !self.sleeping && !self.paused;
        let stopped: Vec<String> =
            self.open_media.keys().filter(|a| !screen_on || !self.playing.contains(*a)).cloned().collect();
        for app in stopped {
            if let Some(row) = self.open_media.remove(&app) {
                db.extend_usage(row, now)?;
            }
        }
        if screen_on {
            for app in &self.playing {
                if !self.open_media.contains_key(app) {
                    let row = db.open_media(now, app)?;
                    self.open_media.insert(app.clone(), row);
                }
            }
        }
        Ok(())
    }

    pub fn on_media(&mut self, db: &Db, now: i64, playing: HashSet<String>) -> Result<()> {
        if playing == self.playing {
            return Ok(());
        }
        self.playing = playing;
        self.sync_media(db, now)
    }

    /// Opens/closes rows so the database matches the current state. `end` is the time
    /// used to close rows that are ending.
    fn sync(&mut self, db: &Db, now: i64, end: i64) -> Result<()> {
        let want_app = self.is_active().then(|| self.focus.clone()).flatten();
        let want_web = want_app.as_ref().and_then(|a| Some((a.clone(), self.current_domain()?.to_string())));

        if self.open_app.as_ref().map(|o| &o.app) != want_app.as_ref() {
            if let Some(o) = self.open_app.take() {
                db.extend_usage(o.row, end)?;
            }
            if let Some(app) = want_app {
                let row = db.open_usage(now, &app, None)?;
                self.open_app = Some(Open { row, app, domain: None });
            }
        }
        let open_web_key = self.open_web.as_ref().map(|o| (o.app.clone(), o.domain.clone().unwrap_or_default()));
        if open_web_key != want_web {
            if let Some(o) = self.open_web.take() {
                db.extend_usage(o.row, end)?;
            }
            if let Some((app, domain)) = want_web {
                let row = db.open_usage(now, &app, Some(&domain))?;
                self.open_web = Some(Open { row, app, domain: Some(domain) });
            }
        }
        Ok(())
    }

    pub fn on_focus(&mut self, db: &Db, now: i64, app: Option<String>) -> Result<()> {
        let app = app.filter(|a| !screentime_core::is_ignored_app(a)).map(|a| {
            // Nautilus windows can report their location ("location:trash:///") as the app id.
            if a.starts_with("location:") { "org.gnome.Nautilus.desktop".to_string() } else { a }
        });
        if app == self.focus {
            return Ok(());
        }
        self.focus = app;
        if let (Some(p), Some(app)) = (&self.pickup, &self.focus) {
            if p.deadline.is_none_or(|d| now <= d) {
                db.set_pickup_app(p.id, app)?;
            }
            if p.deadline.is_none_or(|d| now > d) {
                self.pickup = None;
            }
        }
        self.sync(db, now, now)
    }

    pub fn on_web(&mut self, db: &Db, now: i64, family: &str, state: WebState) -> Result<()> {
        self.web.insert(family.to_string(), state);
        self.sync(db, now, now)
    }

    /// `idle_ms` is the time since the last input event.
    pub fn on_idle_time(&mut self, db: &Db, _rules: &Rules, now: i64, idle_ms: i64) -> Result<()> {
        let idle = idle_ms >= IDLE_MS && !self.media_playing();
        if idle == self.idle {
            return Ok(());
        }
        self.idle = idle;
        // Going idle: the user actually left when input stopped, so back-date the end.
        let end = if idle { now - idle_ms } else { now };
        self.sync(db, now, end)
    }

    pub fn on_lock(&mut self, db: &Db, now: i64, locked: bool) -> Result<()> {
        if locked == self.locked {
            return Ok(());
        }
        self.locked = locked;
        if !locked {
            let (id, _) = db.add_pickup(now, "unlock", self.focus.as_deref())?;
            // With nothing focused, the next focused app is the first one, however late.
            let deadline = self.focus.is_some().then_some(now + PICKUP_GRACE_MS);
            self.pickup = Some(PendingPickup { id, deadline });
            self.idle = false;
        }
        self.sync(db, now, now)?;
        self.sync_media(db, now)
    }

    pub fn on_pause(&mut self, db: &Db, now: i64, paused: bool) -> Result<()> {
        self.paused = paused;
        self.sync(db, now, now)?;
        self.sync_media(db, now)
    }

    pub fn on_sleep(&mut self, db: &Db, now: i64, sleeping: bool) -> Result<()> {
        self.sleeping = sleeping;
        if !sleeping {
            self.idle = false;
        }
        self.sync(db, now, now)?;
        self.sync_media(db, now)
    }

    /// Records the session login as a pickup (idempotent across daemon restarts).
    pub fn on_login(&mut self, db: &Db, login_ms: i64) -> Result<()> {
        let (id, created) = db.add_pickup(login_ms, "login", self.focus.as_deref())?;
        if created && self.focus.is_none() {
            self.pickup = Some(PendingPickup { id, deadline: None });
        }
        Ok(())
    }

    /// Persists progress of open rows so a crash loses at most one heartbeat.
    pub fn heartbeat(&mut self, db: &Db, now: i64) -> Result<()> {
        for o in [&self.open_app, &self.open_web].into_iter().flatten() {
            db.extend_usage(o.row, now)?;
        }
        for row in self.open_media.values() {
            db.extend_usage(*row, now)?;
        }
        Ok(())
    }

    pub fn shutdown(&mut self, db: &Db, now: i64) -> Result<()> {
        self.heartbeat(db, now)?;
        self.open_app = None;
        self.open_web = None;
        self.open_media.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: i64 = 60_000;

    fn rows(db: &Db) -> Vec<(i64, i64, bool, String, Option<String>)> {
        db.usage_between(0, i64::MAX)
            .unwrap()
            .into_iter()
            .filter(|r| r.kind != Kind::Media)
            .map(|r| (r.start_ms, r.end_ms, r.kind == Kind::Web, r.app_id, r.domain))
            .collect()
    }

    #[test]
    fn focus_idle_lock_cycle() {
        let db = Db::open_in_memory().unwrap();
        let rules = Rules::builtin();
        let mut t = Tracker::default();
        t.on_focus(&db, 0, Some("code.desktop".into())).unwrap();
        t.on_focus(&db, 10 * M, Some("slack.desktop".into())).unwrap();
        // Idle detected at 20m after 3m without input: Slack ends at 17m.
        t.on_idle_time(&db, &rules, 20 * M, 3 * M).unwrap();
        t.on_idle_time(&db, &rules, 25 * M, 0).unwrap();
        t.on_lock(&db, 30 * M, true).unwrap();
        t.on_lock(&db, 40 * M, false).unwrap();
        t.shutdown(&db, 45 * M).unwrap();
        assert_eq!(
            rows(&db),
            vec![
                (0, 10 * M, false, "code.desktop".into(), None),
                (10 * M, 17 * M, false, "slack.desktop".into(), None),
                (25 * M, 30 * M, false, "slack.desktop".into(), None),
                (40 * M, 45 * M, false, "slack.desktop".into(), None),
            ]
        );
        let p = db.pickups_between(0, i64::MAX).unwrap();
        assert_eq!(p, vec![(40 * M, "unlock".into(), Some("slack.desktop".into()))]);
    }

    #[test]
    fn pickup_first_app_updates_within_grace() {
        let db = Db::open_in_memory().unwrap();
        let mut t = Tracker::default();
        t.on_focus(&db, 0, Some("code.desktop".into())).unwrap();
        t.on_lock(&db, M, true).unwrap();
        t.on_lock(&db, 2 * M, false).unwrap();
        t.on_focus(&db, 2 * M + 2_000, Some("discord.desktop".into())).unwrap();
        t.on_focus(&db, 3 * M, Some("code.desktop".into())).unwrap();
        let p = db.pickups_between(0, i64::MAX).unwrap();
        assert_eq!(p[0].2.as_deref(), Some("discord.desktop"));
    }

    #[test]
    fn login_pickup_takes_first_focus() {
        let db = Db::open_in_memory().unwrap();
        let mut t = Tracker::default();
        t.on_login(&db, 0).unwrap();
        t.on_focus(&db, 5 * M, Some("code.desktop".into())).unwrap();
        // Restarting the daemon must not add a second login pickup.
        Tracker::default().on_login(&db, 0).unwrap();
        let p = db.pickups_between(0, i64::MAX).unwrap();
        assert_eq!(p, vec![(0, "login".into(), Some("code.desktop".into()))]);
    }

    #[test]
    fn media_plays_in_background_until_lock() {
        let db = Db::open_in_memory().unwrap();
        let rules = Rules::builtin();
        let mut t = Tracker::default();
        t.on_focus(&db, 0, Some("vlc.desktop".into())).unwrap();
        t.on_media(&db, 0, HashSet::from(["vlc.desktop".to_string()])).unwrap();
        // Watching without touching anything: not idle, because the focused app plays.
        t.on_idle_time(&db, &rules, 10 * M, 10 * M).unwrap();
        assert!(!t.idle);
        // Switch to the browser; VLC keeps playing in the background.
        t.on_focus(&db, 10 * M, Some("brave-browser.desktop".into())).unwrap();
        t.on_lock(&db, 36 * M, true).unwrap();
        let media: Vec<_> = db.usage_between(0, i64::MAX).unwrap().into_iter().filter(|r| r.kind == Kind::Media).collect();
        assert_eq!(media.len(), 1);
        assert_eq!((media[0].start_ms, media[0].end_ms), (0, 36 * M));
        // Unlocking while still playing resumes it.
        t.on_lock(&db, 40 * M, false).unwrap();
        t.shutdown(&db, 41 * M).unwrap();
        let media = db.usage_between(0, i64::MAX).unwrap().into_iter().filter(|r| r.kind == Kind::Media).count();
        assert_eq!(media, 2);
    }

    #[test]
    fn browser_sites_and_audible_media() {
        let db = Db::open_in_memory().unwrap();
        let rules = Rules::builtin();
        let mut t = Tracker::default();
        let yt = WebState { domain: Some("youtube.com".into()), audible: true };
        t.on_web(&db, 0, "brave", yt).unwrap();
        t.on_focus(&db, 0, Some("brave-browser.desktop".into())).unwrap();
        // Watching a video without touching input still counts.
        t.on_idle_time(&db, &rules, 10 * M, 10 * M).unwrap();
        assert!(!t.idle);
        t.on_web(&db, 12 * M, "brave", WebState { domain: Some("github.com".into()), audible: false }).unwrap();
        t.on_focus(&db, 20 * M, Some("code.desktop".into())).unwrap();
        let r = rows(&db);
        assert!(r.contains(&(0, 20 * M, false, "brave-browser.desktop".into(), None)));
        assert!(r.contains(&(0, 12 * M, true, "brave-browser.desktop".into(), Some("youtube.com".into()))));
        assert!(r.contains(&(12 * M, 20 * M, true, "brave-browser.desktop".into(), Some("github.com".into()))));
    }
}

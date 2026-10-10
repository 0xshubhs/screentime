//! SQLite store. All timestamps are Unix milliseconds.
//!
//! The database lives in the user's data directory, independent of the installed binaries,
//! so reinstalls and upgrades keep all history. Schema changes are numbered migrations,
//! each applied in one transaction together with its `user_version` bump, after a backup.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

pub use rusqlite::Error;
pub type Result<T> = rusqlite::Result<T>;

/// (slot_ms, iface, kind, rx, tx)
pub type IfaceRow = (i64, String, String, u64, u64);

pub const SLOT_MS: i64 = 15 * 60 * 1000;

/// Schema migrations in order; `PRAGMA user_version` records how many are applied.
/// Never edit a released migration: add a new file instead.
const MIGRATIONS: &[&str] = &[
    include_str!("migrations/0001_initial.sql"),
    include_str!("migrations/0002_media_usage.sql"),
];

/// Pre-migration backups kept next to the database.
const BACKUPS_KEPT: usize = 5;

pub struct Db {
    conn: Connection,
}

/// `$XDG_DATA_HOME/screentime/screentime.db`.
pub fn default_path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"));
    base.join("screentime/screentime.db")
}

pub(crate) fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| "/".into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The focused app while the user is active.
    App,
    /// The site in the focused browser tab.
    Web,
    /// An app playing media, whether or not it is focused.
    Media,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageRow {
    pub start_ms: i64,
    pub end_ms: i64,
    pub kind: Kind,
    pub app_id: String,
    pub domain: Option<String>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?, Some(path))
    }

    /// Read-only access for reporting (the CLI, or the dashboard container).
    pub fn open_readonly(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?, None)
    }

    fn init(conn: Connection, path: Option<&Path>) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // WAL lets readers (dashboard, CLI) run alongside the writer; FULL syncs the WAL on
        // every commit, so a committed write survives a crash or power loss.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        migrate(&conn, path)?;
        Ok(Self { conn })
    }

    /// Schema version (number of migrations applied).
    pub fn schema_version(&self) -> Result<i64> {
        self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))
    }

    /// Runs `f` in one transaction: all of its writes commit together or none do.
    pub fn atomically<T>(&self, f: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        let tx = self.conn.unchecked_transaction()?;
        let v = f(self)?;
        tx.commit()?;
        Ok(v)
    }

    pub fn open_media(&self, now: i64, app_id: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO usage (start_ms, end_ms, kind, app_id) VALUES (?1, ?1, 'media', ?2)",
            params![now, app_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn open_usage(&self, now: i64, app_id: &str, domain: Option<&str>) -> Result<i64> {
        let kind = if domain.is_some() { "web" } else { "app" };
        self.conn.execute(
            "INSERT INTO usage (start_ms, end_ms, kind, app_id, domain) VALUES (?1, ?1, ?2, ?3, ?4)",
            params![now, kind, app_id, domain],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Moves the end of an open interval forward (never before its start).
    pub fn extend_usage(&self, id: i64, end_ms: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE usage SET end_ms = MAX(start_ms, ?2) WHERE id = ?1",
            params![id, end_ms],
        )?;
        Ok(())
    }

    /// Inserts a pickup unless one with the same timestamp and kind exists. Returns its id
    /// and whether it was newly created.
    pub fn add_pickup(&self, ts_ms: i64, kind: &str, first_app: Option<&str>) -> Result<(i64, bool)> {
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO pickups (ts_ms, kind, first_app) VALUES (?1, ?2, ?3)",
            params![ts_ms, kind, first_app],
        )?;
        let id = self.conn.query_row(
            "SELECT id FROM pickups WHERE ts_ms = ?1 AND kind = ?2",
            params![ts_ms, kind],
            |r| r.get(0),
        )?;
        Ok((id, n == 1))
    }

    pub fn set_pickup_app(&self, id: i64, app_id: &str) -> Result<()> {
        self.conn.execute("UPDATE pickups SET first_app = ?2 WHERE id = ?1", params![id, app_id])?;
        Ok(())
    }

    pub fn add_notification(&self, ts_ms: i64, app_id: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO notifications (ts_ms, app_id) VALUES (?1, ?2)",
            params![ts_ms, app_id],
        )?;
        Ok(())
    }

    pub fn add_net_app(&self, ts_ms: i64, app_id: &str, rx: u64, tx: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO net_app (slot_ms, app_id, rx, tx) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (slot_ms, app_id) DO UPDATE SET rx = rx + ?3, tx = tx + ?4",
            params![slot(ts_ms), app_id, rx as i64, tx as i64],
        )?;
        Ok(())
    }

    pub fn add_net_iface(&self, ts_ms: i64, iface: &str, kind: &str, rx: u64, tx: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO net_iface (slot_ms, iface, kind, rx, tx) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (slot_ms, iface) DO UPDATE SET rx = rx + ?4, tx = tx + ?5",
            params![slot(ts_ms), iface, kind, rx as i64, tx as i64],
        )?;
        Ok(())
    }

    pub fn usage_between(&self, from: i64, to: i64) -> Result<Vec<UsageRow>> {
        let mut st = self.conn.prepare_cached(
            "SELECT start_ms, end_ms, kind, app_id, domain FROM usage
             WHERE start_ms < ?2 AND end_ms > ?1 ORDER BY start_ms",
        )?;
        let rows = st.query_map(params![from, to], |r| {
            Ok(UsageRow {
                start_ms: r.get(0)?,
                end_ms: r.get(1)?,
                kind: match r.get::<_, String>(2)?.as_str() {
                    "web" => Kind::Web,
                    "media" => Kind::Media,
                    _ => Kind::App,
                },
                app_id: r.get(3)?,
                domain: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// (ts_ms, kind, first_app)
    pub fn pickups_between(&self, from: i64, to: i64) -> Result<Vec<(i64, String, Option<String>)>> {
        let mut st = self.conn.prepare_cached(
            "SELECT ts_ms, kind, first_app FROM pickups WHERE ts_ms >= ?1 AND ts_ms < ?2 ORDER BY ts_ms",
        )?;
        let rows = st.query_map(params![from, to], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect()
    }

    pub fn notifications_between(&self, from: i64, to: i64) -> Result<Vec<(i64, String)>> {
        let mut st = self.conn.prepare_cached(
            "SELECT ts_ms, app_id FROM notifications WHERE ts_ms >= ?1 AND ts_ms < ?2 ORDER BY ts_ms",
        )?;
        let rows = st.query_map(params![from, to], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// (slot_ms, app_id, rx, tx)
    pub fn net_apps_between(&self, from: i64, to: i64) -> Result<Vec<(i64, String, u64, u64)>> {
        let mut st = self.conn.prepare_cached(
            "SELECT slot_ms, app_id, rx, tx FROM net_app WHERE slot_ms >= ?1 AND slot_ms < ?2",
        )?;
        let rows = st.query_map(params![from, to], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? as u64, r.get::<_, i64>(3)? as u64))
        })?;
        rows.collect()
    }

    pub fn net_ifaces_between(&self, from: i64, to: i64) -> Result<Vec<IfaceRow>> {
        let mut st = self.conn.prepare_cached(
            "SELECT slot_ms, iface, kind, rx, tx FROM net_iface WHERE slot_ms >= ?1 AND slot_ms < ?2",
        )?;
        let rows = st.query_map(params![from, to], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? as u64, r.get::<_, i64>(4)? as u64))
        })?;
        rows.collect()
    }

    pub fn last_pickup(&self) -> Result<Option<(i64, String)>> {
        self.conn
            .query_row("SELECT ts_ms, kind FROM pickups ORDER BY ts_ms DESC LIMIT 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()
    }
}

/// Brings the schema up to date. Each pending migration and its version bump run in a single
/// transaction (SQLite DDL is transactional), so an interrupted upgrade rolls back cleanly.
fn migrate(conn: &Connection, path: Option<&Path>) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let target = MIGRATIONS.len() as i64;
    if current > target {
        // Written by a newer screentime; refuse rather than corrupt it.
        return Err(Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_MISMATCH),
            Some(format!("database schema v{current} is newer than this build (v{target}); upgrade screentime")),
        ));
    }
    if current == target {
        return Ok(());
    }
    let has_data: bool =
        conn.query_row("SELECT count(*) > 0 FROM sqlite_master WHERE type = 'table'", [], |r| r.get(0))?;
    if let (Some(path), true) = (path, has_data) {
        backup(conn, path, current)?;
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

/// Consistent copy of the database (`VACUUM INTO`) before changing its schema, e.g.
/// `screentime.db.v1-1791700000.bak`. Keeps the newest few.
fn backup(conn: &Connection, path: &Path, version: i64) -> Result<()> {
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("screentime.db");
    let dest = path.with_file_name(format!("{name}.v{version}-{ts}.bak"));
    conn.execute("VACUUM INTO ?1", params![dest.to_string_lossy()])?;

    if let Some(dir) = path.parent() {
        let prefix = format!("{name}.v");
        let mut old: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".bak"))
            })
            .collect();
        old.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        let excess = old.len().saturating_sub(BACKUPS_KEPT);
        for p in old.into_iter().take(excess) {
            let _ = std::fs::remove_file(p);
        }
    }
    Ok(())
}

fn slot(ts_ms: i64) -> i64 {
    ts_ms - ts_ms.rem_euclid(SLOT_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("screentime-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("screentime.db")
    }

    #[test]
    fn upgrades_a_v1_database_keeping_every_row() {
        let path = scratch("upgrade");
        {
            // A database as 0.1.0 left it: the initial schema, no user_version.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(MIGRATIONS[0]).unwrap();
            conn.execute("INSERT INTO usage (start_ms, end_ms, kind, app_id) VALUES (1, 2, 'app', 'code.desktop')", []).unwrap();
            conn.execute("INSERT INTO usage (start_ms, end_ms, kind, app_id, domain) VALUES (3, 4, 'web', 'brave-browser.desktop', 'github.com')", []).unwrap();
            conn.execute("INSERT INTO pickups (ts_ms, kind) VALUES (5, 'login')", []).unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len() as i64);
        let rows = db.usage_between(0, 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].domain.as_deref(), Some("github.com"));
        assert_eq!(db.pickups_between(0, 10).unwrap().len(), 1);
        // The new kind is accepted now.
        db.open_media(6, "vlc.desktop").unwrap();
        // A backup of the old file was taken first.
        let backups = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".bak"))
            .count();
        assert_eq!(backups, 1);
        // Reopening is a no-op: no second backup, same data.
        drop(db);
        let db = Db::open(&path).unwrap();
        assert_eq!(db.usage_between(0, 10).unwrap().len(), 3);
    }

    #[test]
    fn refuses_a_newer_schema() {
        let path = scratch("newer");
        Connection::open(&path).unwrap().pragma_update(None, "user_version", 99).unwrap();
        assert!(Db::open(&path).is_err());
    }

    #[test]
    fn atomically_rolls_back_on_error() {
        let db = Db::open_in_memory().unwrap();
        let r: Result<()> = db.atomically(|db| {
            db.add_notification(1, "a.desktop")?;
            db.conn.execute("INSERT INTO nope VALUES (1)", [])?;
            Ok(())
        });
        assert!(r.is_err());
        assert!(db.notifications_between(0, 10).unwrap().is_empty());
    }
}

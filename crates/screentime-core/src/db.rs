//! SQLite store. All timestamps are Unix milliseconds.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

pub use rusqlite::Error;
pub type Result<T> = rusqlite::Result<T>;

/// (slot_ms, iface, kind, rx, tx)
pub type IfaceRow = (i64, String, String, u64, u64);

pub const SLOT_MS: i64 = 15 * 60 * 1000;

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

#[derive(Debug, Clone, PartialEq)]
pub struct UsageRow {
    pub start_ms: i64,
    pub end_ms: i64,
    pub web: bool,
    pub app_id: String,
    pub domain: Option<String>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?)
    }

    /// Read-only access for reporting (the CLI, or the dashboard container).
    pub fn open_readonly(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(crate::SCHEMA)?;
        Ok(Self { conn })
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
                web: r.get::<_, String>(2)? == "web",
                app_id: r.get(3)?,
                domain: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Total foreground app time in [from, to), clipped to the range.
    pub fn app_total_between(&self, from: i64, to: i64) -> Result<i64> {
        self.conn.query_row(
            "SELECT COALESCE(SUM(MIN(end_ms, ?2) - MAX(start_ms, ?1)), 0) FROM usage
             WHERE kind = 'app' AND start_ms < ?2 AND end_ms > ?1",
            params![from, to],
            |r| r.get(0),
        )
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

fn slot(ts_ms: i64) -> i64 {
    ts_ms - ts_ms.rem_euclid(SLOT_MS)
}

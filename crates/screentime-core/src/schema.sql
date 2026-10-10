PRAGMA journal_mode = WAL;

-- One row per continuous span of focused, non-idle use. kind = 'app' rows are the source of
-- truth for totals; kind = 'web' rows break browser time down by site.
CREATE TABLE IF NOT EXISTS usage (
    id       INTEGER PRIMARY KEY,
    start_ms INTEGER NOT NULL,
    end_ms   INTEGER NOT NULL,
    kind     TEXT    NOT NULL CHECK (kind IN ('app', 'web')),
    app_id   TEXT    NOT NULL,
    domain   TEXT
);
CREATE INDEX IF NOT EXISTS usage_start ON usage (start_ms);

-- A login or an unlock, plus the first app used afterwards.
CREATE TABLE IF NOT EXISTS pickups (
    id        INTEGER PRIMARY KEY,
    ts_ms     INTEGER NOT NULL,
    kind      TEXT    NOT NULL CHECK (kind IN ('login', 'unlock')),
    first_app TEXT,
    UNIQUE (ts_ms, kind)
);

CREATE TABLE IF NOT EXISTS notifications (
    id     INTEGER PRIMARY KEY,
    ts_ms  INTEGER NOT NULL,
    app_id TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS notifications_ts ON notifications (ts_ms);

-- Network bytes in 15 minute slots (aligns with every UTC offset).
CREATE TABLE IF NOT EXISTS net_app (
    slot_ms INTEGER NOT NULL,
    app_id  TEXT    NOT NULL,
    rx      INTEGER NOT NULL DEFAULT 0,
    tx      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (slot_ms, app_id)
);
CREATE TABLE IF NOT EXISTS net_iface (
    slot_ms INTEGER NOT NULL,
    iface   TEXT    NOT NULL,
    kind    TEXT    NOT NULL,
    rx      INTEGER NOT NULL DEFAULT 0,
    tx      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (slot_ms, iface)
);

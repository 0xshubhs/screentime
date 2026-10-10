CREATE TABLE IF NOT EXISTS usage (
    id       INTEGER PRIMARY KEY,
    start_ms INTEGER NOT NULL,
    end_ms   INTEGER NOT NULL,
    kind     TEXT    NOT NULL CHECK (kind IN ('app', 'web')),
    app_id   TEXT,
    domain   TEXT,
    title    TEXT,
    category TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS usage_start ON usage (start_ms);

CREATE TABLE IF NOT EXISTS pickups (
    ts_ms INTEGER NOT NULL
);

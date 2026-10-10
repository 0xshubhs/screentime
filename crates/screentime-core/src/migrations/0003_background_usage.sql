-- 0003: usage.kind 'media' becomes 'background': an app in use without focus, because it
-- plays audio/video or because it is the top window on another display. Rebuilds the table
-- (SQLite cannot alter a CHECK constraint) and converts existing rows. Runs inside one
-- transaction with the user_version bump, so it applies fully or not at all.

CREATE TABLE usage_new (
    id       INTEGER PRIMARY KEY,
    start_ms INTEGER NOT NULL,
    end_ms   INTEGER NOT NULL,
    kind     TEXT    NOT NULL CHECK (kind IN ('app', 'web', 'background')),
    app_id   TEXT    NOT NULL,
    domain   TEXT
);

INSERT INTO usage_new (id, start_ms, end_ms, kind, app_id, domain)
    SELECT id, start_ms, end_ms, CASE kind WHEN 'media' THEN 'background' ELSE kind END, app_id, domain
    FROM usage;

DROP TABLE usage;
ALTER TABLE usage_new RENAME TO usage;
CREATE INDEX usage_start ON usage (start_ms);

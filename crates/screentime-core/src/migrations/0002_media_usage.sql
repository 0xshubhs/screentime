-- 0002: allow usage.kind = 'media' (an app playing audio/video, focused or not).
-- SQLite cannot alter a CHECK constraint, so rebuild the table the documented way:
-- create the new table, copy every row, drop the old one, rename, recreate the index.
-- Runs inside one transaction with the user_version bump, so it applies fully or not at all.

CREATE TABLE usage_new (
    id       INTEGER PRIMARY KEY,
    start_ms INTEGER NOT NULL,
    end_ms   INTEGER NOT NULL,
    kind     TEXT    NOT NULL CHECK (kind IN ('app', 'web', 'media')),
    app_id   TEXT    NOT NULL,
    domain   TEXT
);

INSERT INTO usage_new (id, start_ms, end_ms, kind, app_id, domain)
    SELECT id, start_ms, end_ms, kind, app_id, domain FROM usage;

DROP TABLE usage;
ALTER TABLE usage_new RENAME TO usage;
CREATE INDEX usage_start ON usage (start_ms);

-- How long a project with no shell in it keeps running, or never. Seeded
-- explicitly to 30 minutes, the default every database starts with.
ALTER TABLE settings ADD COLUMN idle_stop_kind TEXT NOT NULL DEFAULT 'after'
    CHECK (idle_stop_kind IN ('after', 'never'));
ALTER TABLE settings ADD COLUMN idle_stop_ms INTEGER DEFAULT 1800000
    CHECK ((idle_stop_kind = 'after') = (idle_stop_ms IS NOT NULL)
       AND (idle_stop_ms IS NULL OR idle_stop_ms > 0));
UPDATE settings SET idle_stop_kind = 'after', idle_stop_ms = 1800000 WHERE id = 1;

-- A project's own idle stop; both NULL follows the setting.
ALTER TABLE projects ADD COLUMN idle_stop_kind TEXT
    CHECK (idle_stop_kind IN ('after', 'never'));
ALTER TABLE projects ADD COLUMN idle_stop_ms INTEGER
    CHECK ((idle_stop_kind IS 'after') = (idle_stop_ms IS NOT NULL)
       AND (idle_stop_ms IS NULL OR idle_stop_ms > 0));

-- TCP ports a process listens on, checked free before it starts.
CREATE TABLE process_ports (
    process_id INTEGER NOT NULL REFERENCES processes(id) ON DELETE CASCADE,
    port       INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    PRIMARY KEY (process_id, port)
) WITHOUT ROWID;

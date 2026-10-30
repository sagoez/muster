-- When the last shell left each project whose idle stop is pending, in
-- Unix milliseconds, so a service started later stops it on time.
CREATE TABLE pending_idle_stops (
    project_id INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    left_at_ms INTEGER NOT NULL CHECK (left_at_ms >= 0)
);

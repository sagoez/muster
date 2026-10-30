-- How each process's last run ended on its own, kept while it does not run,
-- so a service started later still shows it crashed or exited with its last
-- exit. A start or a stop on request clears it; removing the process removes
-- it. An exit has a status, a signal, or neither when it was not observed.
CREATE TABLE process_runs (
    process_id  INTEGER PRIMARY KEY REFERENCES processes(id) ON DELETE CASCADE,
    exit_status INTEGER CHECK (exit_status >= 0),
    exit_signal TEXT    CHECK (length(exit_signal) > 0),
    ended_at_ms INTEGER NOT NULL,
    CHECK (exit_status IS NULL OR exit_signal IS NULL)
);

-- The newest agent edit of each project a service raised as an event, so a
-- later service raises only the edits no service did.
CREATE TABLE raised_edits (
    project_id INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    edit_id    INTEGER NOT NULL
);

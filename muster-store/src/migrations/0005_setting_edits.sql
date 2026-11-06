-- An agent's change to a machine-wide setting is kept in every project's
-- edit log, and concerns no process, so `process` may be NULL.
CREATE TABLE stack_edits_new (
    id         INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    at_ms      INTEGER NOT NULL,
    process    TEXT,
    message    TEXT    NOT NULL
);

INSERT INTO stack_edits_new (id, project_id, at_ms, process, message)
    SELECT id, project_id, at_ms, process, message FROM stack_edits;

DROP TABLE stack_edits;

ALTER TABLE stack_edits_new RENAME TO stack_edits;

CREATE INDEX stack_edits_by_project ON stack_edits (project_id, id);

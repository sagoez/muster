-- Each agent edit to a project's stack, as the user is told of it, so
-- `muster notices` and MCP show it whether or not a service ran at the time.
-- The store keeps only the newest entries per project; removing the project
-- removes its log.
CREATE TABLE stack_edits (
    id         INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    at_ms      INTEGER NOT NULL,
    process    TEXT    NOT NULL,
    message    TEXT    NOT NULL
);

CREATE INDEX stack_edits_by_project ON stack_edits (project_id, id);

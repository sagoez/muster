-- Each project's stack revision, bumped by every write to its stack, so the
-- service can tell its loaded stack is stale without being told. Values come
-- from one store-wide counter, so a project removed and registered again never
-- repeats a revision the service still holds for it.
CREATE TABLE stack_revision_counter (
    id    INTEGER PRIMARY KEY CHECK (id = 1),
    value INTEGER NOT NULL
);
INSERT INTO stack_revision_counter (id, value) VALUES (1, 0);

ALTER TABLE projects ADD COLUMN stack_revision INTEGER NOT NULL DEFAULT 0;

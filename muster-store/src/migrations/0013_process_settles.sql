-- When a service last settled a run of each process: saw how it ended,
-- stopped it, or saw it exit while unsupervised. An exit a holder left beside
-- its record for a later service, of a run that began before then, was
-- settled already or overtaken by what the user did since, so it is dropped
-- rather than replayed. Removing the process removes it.
CREATE TABLE process_settles (
    process_id    INTEGER PRIMARY KEY REFERENCES processes(id) ON DELETE CASCADE,
    settled_at_ms INTEGER NOT NULL
);

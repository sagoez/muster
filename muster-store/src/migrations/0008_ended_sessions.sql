-- The MCP connections whose locks were released as they ended. A lock
-- acquire still running when its connection ended checks this in its own
-- transaction, so it can never commit a lock for a session already gone.
-- A row is kept only as long as a lock could be: past the longest TTL it is
-- pruned.
CREATE TABLE ended_sessions (
    session  TEXT    PRIMARY KEY,
    ended_at INTEGER NOT NULL
);

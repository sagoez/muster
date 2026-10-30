-- A lock belongs to the MCP connection that took it, not only to its agent
-- label: renewing and releasing match the connection's session, and a
-- disconnect releases only that session's locks. A lock taken before this
-- migration belongs to no session and lapses by its TTL.
ALTER TABLE locks ADD COLUMN session TEXT NOT NULL DEFAULT '';

CREATE INDEX locks_by_session ON locks (session);

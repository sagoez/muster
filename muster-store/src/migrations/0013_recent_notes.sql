-- A project's notes, the most recently written first, as `muster mcp context`
-- reads them: the index walks them in that order, rowid breaking ties, so
-- the read stops at its limit instead of sorting every note first.
CREATE INDEX scratchpads_recent ON scratchpads (project_id, updated_at);

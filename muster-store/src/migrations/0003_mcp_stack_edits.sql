-- Whether an agent over MCP may add, change or remove processes. Seeded
-- explicitly to 'allow', the default every new database starts with.
ALTER TABLE settings ADD COLUMN mcp_stack_edits TEXT NOT NULL DEFAULT 'allow'
    CHECK (mcp_stack_edits IN ('allow', 'read-only'));
UPDATE settings SET mcp_stack_edits = 'allow' WHERE id = 1;

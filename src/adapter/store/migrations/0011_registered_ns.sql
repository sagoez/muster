-- When each project was registered, in Unix nanoseconds, so a shell's report
-- can tell a project registered after its prompt, around the shell, from
-- one it walked into.
ALTER TABLE projects RENAME COLUMN created_at TO registered_ns;
UPDATE projects SET registered_ns = registered_ns * 1000000000;

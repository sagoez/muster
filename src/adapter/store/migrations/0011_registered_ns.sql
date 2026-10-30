-- When each project was registered, in Unix nanoseconds, and the hooked shell
-- that registered it, by pid and start token, when one did, so a shell's
-- report can tell a project registered around the shell from one it walked
-- into.
ALTER TABLE projects RENAME COLUMN created_at TO registered_ns;
UPDATE projects SET registered_ns = registered_ns * 1000000000;
ALTER TABLE projects ADD COLUMN registered_by_pid INTEGER CHECK (registered_by_pid > 0);
ALTER TABLE projects ADD COLUMN registered_by_start INTEGER CHECK (registered_by_start > 0);

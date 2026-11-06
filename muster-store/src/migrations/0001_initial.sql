CREATE TABLE projects (
    id         INTEGER PRIMARY KEY,
    root       TEXT    NOT NULL UNIQUE,
    name       TEXT    NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
);

CREATE TABLE processes (
    id               INTEGER PRIMARY KEY,
    project_id       INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name             TEXT    NOT NULL,
    name_key         TEXT    NOT NULL,
    command          TEXT    NOT NULL CHECK (length(trim(command)) > 0),
    working_dir      TEXT,
    description      TEXT,
    autostart        INTEGER NOT NULL CHECK (autostart IN (0, 1)),
    restart          TEXT    NOT NULL CHECK (restart IN ('never', 'on-failure', 'always')),
    stop_signal      TEXT    NOT NULL CHECK (stop_signal IN ('terminate', 'interrupt')),
    stop_grace_ms    INTEGER NOT NULL CHECK (stop_grace_ms > 0),
    ready_kind       TEXT    CHECK (ready_kind IN ('log', 'regex', 'port', 'http')),
    ready_target     TEXT,
    ready_timeout_ms INTEGER CHECK (ready_timeout_ms > 0),
    UNIQUE (project_id, name_key),
    CHECK ((ready_kind IS NULL) = (ready_target IS NULL)
       AND (ready_kind IS NULL) = (ready_timeout_ms IS NULL))
);
CREATE INDEX processes_in_order ON processes (project_id, id);

CREATE TABLE process_env (
    process_id INTEGER NOT NULL REFERENCES processes(id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,
    value      TEXT    NOT NULL,
    PRIMARY KEY (process_id, name)
) WITHOUT ROWID;

CREATE TABLE settings (
    id                    INTEGER PRIMARY KEY CHECK (id = 1),
    desktop_notifications INTEGER NOT NULL CHECK (desktop_notifications IN (0, 1)),
    default_rows          INTEGER NOT NULL,
    default_cols          INTEGER NOT NULL,
    detach_key            TEXT    NOT NULL,
    log_max_size_bytes    INTEGER NOT NULL CHECK (log_max_size_bytes > 0),
    scrollback_lines      INTEGER NOT NULL CHECK (scrollback_lines >= 0),
    color_scheme          TEXT    NOT NULL CHECK (color_scheme IN ('dark', 'light')),
    default_autostart     INTEGER NOT NULL CHECK (default_autostart IN (0, 1)),
    default_restart       TEXT    NOT NULL CHECK (default_restart IN ('never', 'on-failure', 'always')),
    default_stop_signal   TEXT    NOT NULL CHECK (default_stop_signal IN ('terminate', 'interrupt')),
    default_stop_grace_ms INTEGER NOT NULL CHECK (default_stop_grace_ms > 0)
);
INSERT INTO settings VALUES (1, 1, 24, 80, 'ctrl-\', 10485760, 1000, 'dark', 0, 'never', 'terminate', 5000);

CREATE TABLE scratchpads (
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    key        TEXT    NOT NULL,
    body       TEXT    NOT NULL,
    author     TEXT    NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (project_id, key)
);

CREATE TABLE todos (
    project_id    INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    id            TEXT    NOT NULL,
    title         TEXT    NOT NULL,
    status        TEXT    NOT NULL CHECK (status IN ('pending', 'in-progress', 'done')),
    deps          TEXT    NOT NULL,
    author        TEXT    NOT NULL,
    updated_at    INTEGER NOT NULL,
    owner         TEXT,
    claimed_until INTEGER,
    PRIMARY KEY (project_id, id),
    CHECK ((owner IS NULL) = (claimed_until IS NULL)),
    CHECK (owner IS NULL OR status = 'in-progress')
);

CREATE TABLE kv (
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    key        TEXT    NOT NULL,
    value      TEXT    NOT NULL,
    author     TEXT    NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (project_id, key)
);

CREATE TABLE locks (
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,
    owner      TEXT    NOT NULL,
    note       TEXT,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (project_id, name)
);
CREATE INDEX locks_by_owner ON locks (owner);

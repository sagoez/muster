# muster reference

The details behind the [README](../README.md): how projects are found, every
process field, the stack file, settings, commands, exit codes, JSON output,
the shell hook, ports, attach and open, logs, the service, and the MCP tools.
`muster --help` and `muster <command> --help` are the source of truth for
flags.

## Projects

A project is a folder. The first `muster add`, `run`, `import` or `init` that
adds a process to a folder registers it, named after the folder (`shop`, then
`shop-2` for a second folder of that name, with control and format characters
escaped). `/`, your home folder, a folder above it and a folder holding a
registered project are never registered.

A command acts on the project given by `--project` (a registered name, trimmed,
or a folder, taken exactly, when the value as given contains `/` or starts with
`.` or `~`; an absolute or `~` path led by a space is refused), then
`$MUSTER_PROJECT` (set inside every process muster runs), then the deepest
registered folder containing the current directory. A folder inside a project
is that project, except for `add`, `run`, `init` and `import`, which register
it as a project of its own. When `$MUSTER_PROJECT` points `add`, `set`, `rm`,
`init` or `import` away from the project the current directory is in, the
command says which project it acts on. Bare `muster` is `muster ps` inside a
project and `muster ps --all` outside one.

## Where things live

`muster.db`, a SQLite database in the state directory (`~/.local/state/muster`
on Linux, `~/Library/Application Support/muster` on macOS), readable by its
owner only, holds every project's stack, the settings, the coordination state,
the agent edit log and how each process last ended. Each process's holder
listens on a Unix socket in the state directory.

The service listens in `$XDG_RUNTIME_DIR/muster`, else `/tmp/muster-<uid>`,
which must be yours with mode 0700, and records its socket in
`service.socket` in the state directory so a command started with a different
runtime directory still reaches it. A client talks only to a service running
as you. When another user made the `/tmp` directory, set `XDG_RUNTIME_DIR` to a
directory of your own. A socket path too long to fit is refused with a hint to
set a shorter `XDG_STATE_HOME` or `XDG_RUNTIME_DIR`.

The service writes one JSON line per notice to `service.log` in the state
directory, rotated to `service.log.1` past 1 MiB.

## Processes

```sh
muster add web --cwd frontend --env PORT=5173 --restart on-failure -- npm run dev
muster add worker --stop-signal interrupt --stop-grace 10s -- bin/worker
muster set web --autostart --description "the dev server"
muster rm worker
```

One argument after `--` is a shell expression; several are joined as shell
words. A process has:

- a name, unique in the project, at most 64 characters, with no control or
  format characters; a project holds at most 100 processes;
- a command, run by the shell, at most 8192 characters;
- `--cwd`, where it runs, stored relative to the project folder when inside it;
- `--env KEY=VALUE`, repeatable: at most 64 variables, names up to 128
  characters, values up to 8192;
- `--description`, at most 300 characters;
- `--autostart` / `--no-autostart`, whether `muster up` starts it;
- `--restart`: `never`, `on-failure` or `always`, with backoff;
- `--stop-signal` `terminate` (SIGTERM) or `interrupt` (SIGINT), and
  `--stop-grace`, how long a stop waits before SIGKILL, at most `1h`;
- a readiness check: `--ready-log TEXT` (with `--ready-regex`),
  `--ready-port PORT` or `--ready-http URL` on localhost, and
  `--ready-timeout`. A run that misses it raises a warning and keeps running;
- `--port PORT`, repeatable, the TCP ports it listens on, at most 16 distinct
  ports from 1 to 65535.

A field left out takes the matching `default-*` setting, and the process stores
that value. `muster add` never starts anything.

`muster set <name>` changes only what it is given: `--command` or a new command
after `--`, `--env` and `--unset-env`, and `--no-cwd`, `--no-description`,
`--no-ready` and `--no-ports` to clear. `--port` replaces the stored set. A
running process keeps its current run until `muster restart`, and `set` says
when one is needed. A `set` that changes nothing succeeds and says so
(`"changed": false`). `muster rm` stops each process it removes and waits for
it. A name cannot be changed; remove and add again.

A command that starts processes, and a wait for `running`, holds on until each
child has run for half a second, so one that exits at once is reported crashed
or exited, not started.

## Stack file

`muster export [FILE]` writes the stack to `muster.yml` in the project folder,
or to `FILE` (`-` for stdout), and needs `--force` to overwrite. Working
directories are written relative to the project, and a process working outside
the project is refused until `muster set <name> --cwd` moves it. Environment
variables are written by name with a null placeholder; `--with-env` writes the
values too, to a file only its owner can read.

`muster import [FILE]` loads a stack file into the current folder, registering
it when needed. It refuses names the project already has unless given
`--merge` (replace those) or `--replace` (make the stack exactly the file,
stopping what it removes). `--dry-run` prints the plan. An import reads only a
regular file of at most 1 MiB, and refuses a file that still has an
environment placeholder or whose `working_dir` is empty, absolute, or leads out
of the project through `..` or a symlink.

```yaml
# muster stack: `muster import` loads it into a project.
version: 1
processes:
  - name: web
    command: npm run dev
    working_dir: frontend
    environment:
      PORT: "5173"
    autostart: true
    restart: on-failure
    stop:
      signal: terminate
      grace_period: 5s
    ready:
      log: ready in
      timeout: 60s
    ports:
      - 5173
```

`ready` is optional and takes one of `log` (with `regex: true`), `port` or
`http`, plus a `timeout`. `ports` is optional, ascending, and left out when
empty. The file never names the project: the folder that imports it is the
project. [examples/muster.yml](../examples/muster.yml) is a complete example.

`muster init` reads `package.json` scripts, `Procfile.dev` and `Procfile`,
public `justfile` recipes, `Makefile` targets and compose files, and offers
what it finds with the likely dev commands preselected. It never runs them. At
a terminal it asks; elsewhere it prints the candidates, and `--yes` adds the
suggested ones.

## Settings

`muster config` lists them, `muster config get <key>` prints one, and
`muster config set <key> <value>` changes one.

| Key | Value |
|---|---|
| `desktop-notifications` | `true` or `false`: whether failures raise a desktop notification |
| `default-size` | `ROWSxCOLS`, such as `24x80`: the terminal size a process starts with, 1 to 1000 rows by 2 to 2000 columns |
| `detach-key` | a Control chord, such as `ctrl-\`, that ends `muster attach`; `ctrl-m`, `ctrl-j`, `ctrl-i`, `ctrl-h` and `ctrl-c` are refused |
| `log-max-size` | `16KiB` to `1GiB`, such as `10MiB`: when a process's log rotates |
| `scrollback` | 0 to 100000 lines of history per process terminal |
| `color-scheme` | `dark` or `light`: the colors a process's terminal reports |
| `default-autostart` | `true` or `false` for a new process |
| `default-restart` | `never`, `on-failure` or `always` for a new process |
| `default-stop-signal` | `terminate` or `interrupt` for a new process |
| `default-stop-grace` | a duration up to `1h`, such as `5s`, for a new process |
| `mcp-stack-edits` | `allow` (default) or `read-only`: whether an agent may add, change or remove processes |
| `idle-stop` | a duration such as `30m` (default), or `never`: how long a project keeps running once no shell is in it |

`detach-key` applies at the next attach; `desktop-notifications`,
`mcp-stack-edits` and `idle-stop` at once; the size, scrollback, log size and
color scheme to processes started afterwards; the `default-*` settings to
processes added afterwards. Over MCP, `config_set` changes only the `default-*`
settings.

## Commands

| Command | What it does |
|---|---|
| `muster ps [--all] [--commands]` | List processes with state, readiness, listening ports, pid, uptime, restarts and usage; the heading shows the shells in the project or the time left before it stops |
| `muster up` / `down` | Start every autostart process / stop every process |
| `muster start [name] [--wait [--timeout D]]` | Start a process, optionally waiting until it is ready |
| `muster stop [name]` | Stop a process through its stop policy |
| `muster restart [name] [--wait [--timeout D]]` | Restart a process, waiting as `start` does |
| `muster wait [name] [--log P [--regex] \| --port N \| --http URL [--status C] \| --state S] [--since CURSOR] [--timeout D]` | Wait for a line, a connection, an HTTP answer, a state, or the readiness check |
| `muster pause [name]` / `resume [name]` | Suspend and resume a process (SIGSTOP, SIGCONT) |
| `muster kill [name]` | Kill a process at once |
| `muster logs [name] [-f] [--screen] [-n N] [--since CURSOR] [--grep P [--regex]] [--errors]` | Print, follow or filter a process's log, or print its screen |
| `muster attach [name] [--read-only]` | Show a running process in this terminal and type into it |
| `muster open [name] [--read-only] [--direction right\|down]` | Show a running process in a new herdr pane beside this one; outside herdr, `attach` |
| `muster add <name> [options] -- <command>` | Add a process, registering the folder |
| `muster set <name> [options]` | Change a process's fields |
| `muster rm <name>...` | Remove processes, stopping them first |
| `muster run [--name] -- <command>` | Add a command, start it and attach |
| `muster config [list\|get\|set]` | Read and change settings |
| `muster export [FILE] [--force] [--with-env]` | Write the stack to a file |
| `muster import [FILE] [--merge\|--replace] [--dry-run]` | Load a stack file |
| `muster init [--yes]` | Detect a stack in the folder and add it |
| `muster shell-init <bash\|zsh\|fish>` | Print the shell hook |
| `muster projects` / `projects rm <project>` | List registered projects, or remove one with its processes and coordination state (the folder is untouched) |
| `muster projects set [PROJECT] --idle-stop <DURATION\|never\|default>` | Give a project its own idle stop, or follow the setting again |
| `muster notices` | Show the stack edits agents made, and the failed starts, forced stops and crashes the service recorded; never starts the service |
| `muster service status` / `stop` | Show or stop the service; processes keep running |
| `muster note` / `todo` / `kv` | Read and write the project's shared notes, todos and key-value state, paged with `--limit` (50 by default, at most 200) and `--cursor` |
| `muster lock` / `lock release <name>` | List agents' locks, or clear one left behind |
| `muster coordination export\|import` | Move the coordination state as YAML |
| `muster mcp [--project P] [--as NAME] [--print-config]` | Serve muster to an agent over MCP |
| `muster doctor` | Check the database, projects, stored processes, completions, the shell hook and the terminal workspace |
| `muster completions <shell>` | Print the line that enables completions |

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | A process did not end up as asked, the action failed, or the current state refused it (a taken name, an import conflict, a folder holding a project, a todo another agent holds) |
| 2 | The command line could not be parsed, set named nothing to change, a confirmation is missing, or a question was cancelled |
| 3 | The muster service is unavailable or runs a newer version |
| 4 | An invalid value: not a project, an unknown project or process, a refused argument, or an unusable file |
| 130 | Interrupted with Ctrl-C while it waited at a terminal |

Every invalid value exits 4, whether the argument parser or the command
refuses it. Exit 2 is only for a command line that does not parse. A valid
request the current state refuses exits 1 with the kind `conflict`.

## At a terminal

When stdin, stdout and stderr are all terminals, `--json` is not given,
`NO_COLOR` is unset or empty and `TERM` is not `dumb`, commands that act frame
their output with a spinner, and a command missing a process name offers a
list to pick from. Ctrl-C exits 130; before the request is sent it cancels
with nothing changed. Anywhere else muster prints plain lines and asks nothing:
a question it would have asked fails with exit 4 naming the argument, and
`muster projects rm` needs `--yes`.

## JSON

Every command but `mcp`, `completions` and `shell-init` takes `--json` and
prints results and failures as JSON on stdout. Keys are snake_case, enum values kebab-case
(`on-failure`, `in-progress`), durations are seconds, and times are RFC 3339
in UTC to the second. A setting's `value` is the text `config set` takes, such
as `"5s"`. A process definition:

```json
{"name": "web", "command": "npm run dev", "working_dir": "frontend", "description": null,
 "environment": {"PORT": "5173"}, "autostart": true, "restart": "on-failure",
 "stop": {"signal": "terminate", "grace": 5.0}, "ready": {"port": 5173, "timeout": 30.0},
 "ports": [5173]}
```

A project appears as `{"name": "shop", "root": "/home/me/shop"}`. `muster logs
--json` carries the `cursor` just past the text, to pass as `--since`;
`muster logs -f --json` prints one object per line. The coordination commands
answer in the shapes of the matching MCP tools, and `muster doctor --json`
lists each `check` with its `outcome`.

## Shell hook

| Shell | File | Line |
|---|---|---|
| bash | `~/.bashrc` | `eval "$(muster shell-init bash)"` |
| zsh | `~/.zshrc` | `eval "$(muster shell-init zsh)"` |
| fish | `~/.config/fish/config.fish` | `muster shell-init fish \| source` |

At each prompt the hook runs `muster __shell-report` in the background and
moves on, so a missing binary or a stopped service changes nothing. It exports
`MUSTER_SHELL_PID` and adds no alias or exit trap. Only interactive shells load
it, a shell running as a user who does not own the state directory reports
nothing, and the shell of a pane `muster open` made loads no hook, so it never
occupies the project or holds off its idle stop.

Entering a registered project's folder starts its autostart processes, as
`muster up` would, and the next prompt prints a `muster:` line saying what
happened. Registering a project around a shell that is already in it does not
count as entering. A process you stopped yourself stays stopped.

Leaving is noticed at that shell's next report, and a shell that ends within a
few seconds. Once no shell has been in the project for `idle-stop`, muster
stops its running processes, raises a warning and an `idle-stopped` event, and
leaves them stopped. A shell entering, or a start from the CLI or MCP, cancels
a pending stop. Only a shell seen leaving starts the countdown, so a project
brought up with no shell in it is never stopped for want of one.

`muster projects set --idle-stop 10m` gives one project its own value, and
`--idle-stop default` makes it follow the setting again. `muster doctor` warns
when the shell it runs from has not loaded the hook, with the line to add.

## Ports

Before every start, muster lists the machine's listening sockets once and
refuses the start when a declared port, or the readiness port, is held. The
refusal names the first port held and its holder: the project and process when
muster runs it (with the `muster down` or `muster stop` that frees it), or the
pid and command line otherwise. The process shows as `blocked` in `muster ps`
until a later start finds the ports free. The `PORTS` column lists what the
process tree listens on now, which need not be the declared ports. With
`--json` a process carries `ports` and `blocked_on`.

## Attach and open

`muster attach <name>` takes over the terminal like `tmux attach`: it paints the
screen at once, follows resizes and passes keys through. The detach key is
`ctrl-\` by default. Several clients can attach to one process, and
`--read-only` watches without typing or resizing.

`muster open <name>`, run from a herdr pane, opens a new pane beside it,
focuses it and attaches there, leaving your pane as it was. `--direction
right|down` picks the side. Without it, right when the pane is wider than tall
and each half keeps at least 40 columns, and down otherwise. Detaching closes
the pane, and a failed attach leaves its message on screen there. Only a
running process opens.

muster knows it runs in herdr when `HERDR_PANE_ID` is set, and runs the `herdr`
on `PATH`, or the one in `HERDR_BIN_PATH` when that path is absolute, so
`HERDR_SOCKET_PATH` and `HERDR_SESSION` work as they do for herdr. Each herdr
call runs in a process group of its own and times out after 5 seconds; at the
timeout, on a Ctrl-C, hangup or terminate, including a Ctrl-C taken while a
spinner or prompt is on screen, and when the client of a `muster mcp` server
disconnects, the whole group is killed. When no
direction is given and herdr's layout is malformed or does not list the pane,
the pane splits down; when herdr cannot be reached, times out or refuses the
layout, nothing is split. The new pane runs one typed line, so `open` supports
bash, zsh and fish 3 or later; the line runs only where the pane's own
variables are set, so a copy recalled from history elsewhere does nothing. The
pane runs the muster binary as it is at each open: once an upgrade has replaced
the binary a `muster mcp` server started from, the pane runs the new file at
the same path, and only when no file is left there does `process_open` answer
`workspace-failed` naming it. When herdr cannot be reached or refuses, or hands
back a pane id muster cannot pass back to it, `open` exits 1 with the reason,
names the `muster --project <root> attach -- <name>` that shows the process in
this pane, and never falls back to the current pane on its own. Outside herdr,
`open` is `attach`. `muster doctor` has a `workspace` row.

The processes muster starts never inherit herdr's pane identity
(`HERDR_PANE_ID`, `HERDR_PANE_RUNTIME_ID`, `HERDR_TAB_ID`,
`HERDR_WORKSPACE_ID`), so a script among them is never mistaken for the pane it
was started from.

herdr's prefix key is `ctrl+b` by default; `muster doctor` warns when
`detach-key` is the same chord. A herdr custom command that opens a process in
its own pane should run `muster attach`, since `open` inside that pane would
split a second one:

```toml
[[keys.command]]
key = "prefix+w"
type = "pane"
command = "muster attach web"
```

## Logs

Each process's raw output goes to a log under the state directory, across
restarts and after it exits, rotating at `log-max-size` and keeping one older
file. To a pipe or file, `muster logs` writes it byte for byte. On a terminal
it keeps text, colors and cursor moves but drops queries, terminal strings and
other controls, so a log cannot retitle your terminal or write its clipboard.
If output rotates away before `muster logs -f` reads it, the follow says how
many bytes it lost and carries on.

Every byte has an offset that runs on across restarts and rotations, written
as a cursor such as `o:18234`. `-n N` (100 by default, at most 2000), `--since
CURSOR`, `--grep TEXT` and `--errors` print stripped lines. Waits report the
same cursors, and how long they took as `elapsed_s`.

## The service and holders

The first command that needs the service starts it. A terminate or hangup
signal stops it as `muster service stop` does: it stores what it still has to
record and leaves every process running, within 30 seconds; a second signal
ends it at once. A newer muster binary replaces an older running service in
place.

Every process runs under its own holder. The next service finds the holders,
reconnects and rebuilds each screen before it answers, keeping uptime, restart
counts and attached clients, and carries out any pending restart. A process
removed from its stack while the service was down is stopped when the service
finds it. A process whose holder died, or whose holder cannot be reconnected
to, shows as `unsupervised` and is never started twice: `stop` and `kill`
signal it directly, and the service retries the holder on a backoff timer or
on `start` or `restart`.

## MCP

`muster mcp --print-config` prints the config snippet. Each tool call acts on
the project its `project` argument names, else the one the server's folder,
`--project` or `$MUSTER_PROJECT` resolves to, worked out per call. Every
result about a project names it.

`--as NAME` sets the id on the agent's coordination writes and locks. It
defaults to `MUSTER_AGENT`, then `herdr-pane-<HERDR_PANE_ID>`, then a random
session id, prefixed with the client's name (`claude-code@herdr-pane-3`). A
lock belongs to the connection that took it, and a clean exit releases it.

A typical loop: `stack_status` to see what runs; `process_restart` with
`wait: true` after a code change; `process_logs` with `errors_only: true` when
something breaks, then again with `since` set to the returned cursor.

The tools:

- stack: `stack_status`, `stack_up`, `stack_events` (lifecycle events after a
  cursor, optionally long-polled). There is no `stack_down`;
- processes: `process_start`, `process_stop`, `process_restart`,
  `process_wait`, `process_logs`, `process_screen`, `process_send`,
  `process_open`;
- stack definition: `process_add`, `process_set`, `process_remove`, with
  `ready` and `ports` as on the CLI. `process_add` registers only the server's
  own folder or one inside it;
- settings: `config_get`, and `config_set` for the `default-*` settings;
- coordination: `todo_list`, `todo_add`, `todo_claim`, `todo_update`,
  `todo_delete`, `lock_acquire`, `lock_release`, `note_read`, `note_write`,
  `note_delete`, `kv_read`, `kv_write`, `kv_delete`. A note body holds at most
  65536 characters, a value 4096, a todo title 500, a key 128, and a todo at
  most 32 dependencies. Listings page with `limit` and `cursor`. Setting a todo
  `in-progress` claims it.

Guardrails on stack edits, since a changed command runs with your rights:

- `mcp-stack-edits: read-only` refuses `process_add`, `process_set`,
  `process_remove`, `process_send` and `config_set`. Only the CLI changes it.
  `process_open` is not an edit and is allowed.
- Every agent edit is recorded with the edit itself, the newest 50 per
  project, naming the agent, the process and the fields it changed. It shows in
  `muster notices`, `stack_status` and as a `notice` event.
- A `working_dir` must stay inside the project root, symlinks included.
- The size caps from [Processes](#processes) hold on every write. A stored
  value past them, which only a hand edit can make, keeps the project readable
  but that process never starts until it is fixed.
- One connection holds at most 32 live locks.
- A definition read back lists `environment` by variable name only.

A wait takes one condition: `{"log": "..."}` (with `"regex": true`),
`{"port": 3000}`, `{"http": "http://localhost:3000/health"}` (with an
optional `status`), `{"state": "running"}` or `{"ready": true}`. `port` and
`http` reach loopback hosts only. A stored readiness check has the same shape
plus a `timeout` in seconds. Durations are numbers of seconds, stored as whole
milliseconds.

No call blocks longer than 45 seconds: an unfinished wait answers `waiting`
with a cursor to continue from, and a request with a progress token hears
progress every 5 seconds. A `limit` of 0 or past 200, or a timeout past 45, is
refused, not clamped. Output is stripped of terminal escapes and capped.

Event cursors such as `e:3141592:9921` name the service that numbered them. A
cursor from an earlier service, past the newest event, or behind the 1024 kept
answers at once with `gap: true`, which means re-reading `stack_status`.

Every stack and process result carries `alerts`: crashes, readiness timeouts
and blocked starts since the agent's last call, each reported once, with
`alerts_gap: true` when some were dropped before they were read.

A mistake in a call comes back as a result flagged `isError`, whose `error`
names its `kind` and up to 20 `candidates`. A wait ended by an exit or crash
does the same, with the run's last lines. A wait that ran out of time, and a
lock or todo someone else holds, are not errors. `process_open` resolves the
process first, so an unknown or stopped one gets its own answer. Outside herdr
it resolves the name against the stored stack and never starts the service:
with no service running the process is `not-running`, and for a running one it
answers kind `no-workspace`, telling the agent to ask you to run
`muster --project <root> attach -- <name>` in your own terminal, and
when herdr cannot be reached or refuses it answers `workspace-failed` with
herdr's message and the same command.

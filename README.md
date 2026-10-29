# muster

The supervisor and control plane behind your terminal.

muster runs a project's dev stack: its servers, workers and watchers. A
per-user background service starts them, restarts them, keeps their logs and
screens, and answers a CLI and an MCP server. Your terminal stays your choice:
muster draws no panes, and `muster attach` shows any process in the terminal
you are already in. Agents and interactive shells live in
[herdr](https://herdr.dev), which reaches the stack through muster's MCP server.

**Status: Beta.** Linux and macOS.

![muster demo](docs/muster-demo.gif)

## Install

Requires a recent Rust toolchain.

```sh
cargo install muster-workspace
muster add web -- npm run dev   # register this folder and add a process
muster up                       # start its autostart processes
```

There is no daemon to start. The first `muster` command that needs the service
starts it in the background. A terminate or hangup signal stops it as
`muster service stop` does: what it still had to record is stored first, and
every process keeps running. It writes what it reports, one JSON line per
notice, to `service.log` in the state directory, which moves to
`service.log.1` once it passes 1 MiB.

## Projects and stacks

A project is a folder. The first `muster add`, `muster run`, `muster import` or
`muster init` that adds a process to a folder registers it, named after the
folder (`shop`, then `shop-2` for a second folder of that name). Its stack is
the list of processes it defines, in the order they were added. `/`, your home
folder, a folder above it and a folder holding a registered project are never
registered: run the command inside a project folder, or pass `-p`.

Everything muster stores lives in one SQLite database, `muster.db` in muster's
state directory (`~/.local/state/muster` on Linux,
`~/Library/Application Support/muster` on macOS), readable by its owner only:
every project's stack, the machine's settings and the shared notes, todos and
key-values. The CLI and the MCP server write it directly and tell a running
service what changed, so an edit reaches the service at once and works without
it. The service keeps there how each process last crashed or exited, so `ps`
after a service restart still shows it until the process starts again. Each
process listens on a Unix socket in that directory. The service listens in a
short runtime directory instead, `$XDG_RUNTIME_DIR/muster`, else
`/tmp/muster-<uid>`, which must be yours with mode 0700, on a socket named for
the state directory. It records that socket in `service.socket` in the state
directory, so a command started with another runtime directory, or none, still
reaches it. A client checks the socket's directory is yours with mode 0700
before it connects, and talks only to a service running as you. A socket path
too long to fit is refused naming it, with a hint to set a shorter
`XDG_STATE_HOME` or `XDG_RUNTIME_DIR`.

Commands act on the project given by `--project` (a registered name, or a
folder when the value contains `/` or starts with `.` or `~`; such a value
names a registered project when no such folder exists), then
`$MUSTER_PROJECT` (set inside every process muster runs; a folder, or a
registered project's name when no such folder exists), then the deepest
registered folder containing the current directory. A folder named inside a
project is that project, except for `add`, `run`, `init` and `import`, which
register it as a project of its own. A project takes its folder's name with
any control or format character escaped. When `$MUSTER_PROJECT`
points `add`, `set`, `rm`, `init` or `import` away from the project the current
directory is in, the command says which project it acts on. Bare `muster` is
`muster ps` inside a project and `muster ps --all` outside one.

### Adding and changing processes

```sh
muster add web --cwd frontend --env PORT=5173 --restart on-failure -- npm run dev
muster add worker --stop-signal interrupt --stop-grace 10s -- bin/worker
muster set web --autostart --description "the dev server"
muster rm worker
```

`muster add <name> -- <command>` adds a process. One argument after `--` is
taken as a shell expression; several are joined as shell words. Each process
has:

- a name, unique within the project and free of control and format
  characters (such as bidi overrides and zero-width spaces), that
  every command addresses it by;
- a command, run by the shell;
- `--cwd`, where it runs, stored relative to the project folder when inside it;
  the project folder when left out;
- `--env KEY=VALUE`, repeatable, the variables it runs with;
- `--description`, a one-line note;
- `--autostart` or `--no-autostart`, whether `muster up` starts it;
- `--restart`: `never`, `on-failure` or `always`; restarts back off;
- `--stop-signal`: `terminate` (SIGTERM) or `interrupt` (SIGINT), and
  `--stop-grace`, how long a stop waits before SIGKILL, at most `1h`;
- a readiness check, what each run waits for before it counts as ready: one of
  `--ready-log TEXT` (with `--ready-regex` for a regex), `--ready-port PORT` or
  `--ready-http URL` on localhost, and `--ready-timeout`, how long a run has to
  pass it. A run that misses it raises a warning and keeps running. A start,
  or a restart by policy, whose readiness port another process listens on is
  refused, naming that process: its project and name when muster runs it,
  otherwise its pid and command line. `muster ps` shows the process as
  `blocked` until a later start finds the port free.

A field left out takes the matching `default-*` setting, and the process stores
that value, so changing a default later changes only processes added after it.
`muster add` never starts anything.

`muster set <name>` changes the fields it is given: `--command` or a new command
after `--`, `--no-cwd`, `--no-description` and `--no-ready` to clear, `--env` to
add or replace a variable and `--unset-env` to remove one. A running process keeps its
current run; `muster restart <name>` applies a change to its command, working
directory, environment, stop policy or readiness check, and `set` says so when
one needs it. `muster rm <name>...`
stops each process it removes and waits for it to end. A name cannot be
changed: remove the process and add it again.

### Sharing a stack

`muster export [FILE]` writes the project's stack to `muster.yml` in the project
folder, or to `FILE` (`-` for stdout), and refuses to overwrite a file without
`--force`. Each working directory is written relative to the project, and a
process working outside the project is refused until `muster set <name> --cwd`
moves it under it, so every exported file imports. Environment variables are
written by name with a null placeholder; `--with-env` writes their values too,
to a file only its owner can read. `muster import [FILE]` loads such a file
into the current folder, registering it when needed. By default it refuses
names the project already defines; `--merge` replaces those, `--replace` makes
the stack exactly the file and stops what it removes, and `--dry-run` prints
the plan, with `--json` reporting `registered: false` and whether the import
`would_register` the folder. Both the plan and the report show the command and
working directory of each process added or changed. An import reads only a
regular file of at most 1 MiB, and refuses a file whose environment still
holds a placeholder or whose `working_dir` is empty, absolute, climbs out of
the project with `..`, or leads out of it through a symlink.

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
```

`ready` is optional and takes one of `log` (with `regex: true` for a regex),
`port` or `http`, and a `timeout`.
[examples/muster.yml](examples/muster.yml) is a complete example. The file
never names the project: the folder that imports it is the project.

### Detecting a stack

`muster init` reads `package.json` scripts, `Procfile.dev` and `Procfile` (the
first wins a shared name), public `justfile` recipes, `Makefile` targets and
compose files in the project's folder and offers what it finds, with the likely
dev commands preselected. It names the folder it scanned, and from a subfolder
with files of its own it points at `muster init --project .`. It only reads
files and never runs them. At a terminal it asks which to add; elsewhere it
prints the candidates, and `muster init --yes` adds the suggested ones.

## Settings

`muster config` lists the machine's settings, `muster config get <key>` prints
one and `muster config set <key> <value>` changes it.

| Key | Value |
|---|---|
| `desktop-notifications` | `true` or `false`: whether failures raise a desktop notification |
| `default-size` | `ROWSxCOLS`, such as `24x80`: the terminal size a process starts with, 1 to 1000 rows by 2 to 2000 columns |
| `detach-key` | a Control chord, such as `ctrl-\`: the key that ends `muster attach`; `ctrl-m`, `ctrl-j`, `ctrl-i`, `ctrl-h` and `ctrl-c` are refused, since terminals send them for Enter, Tab, Backspace and interrupt |
| `log-max-size` | a size from `16KiB` to `1GiB`, such as `10MiB`: when a process's log rotates |
| `scrollback` | 0 to 100000 lines of history each process's terminal keeps |
| `color-scheme` | `dark` or `light`: the default colors a process's terminal reports |
| `default-autostart` | `true` or `false` for a new process |
| `default-restart` | `never`, `on-failure` or `always` for a new process |
| `default-stop-signal` | `terminate` or `interrupt` for a new process |
| `default-stop-grace` | a duration up to `1h`, such as `5s`, for a new process |
| `mcp-stack-edits` | `allow` or `read-only`: whether an agent over MCP may add, change or remove processes; `allow` by default |

`detach-key` applies at the next attach, `desktop-notifications` and
`mcp-stack-edits` at once, the size, scrollback, log size and color scheme to
processes started afterwards, and the `default-*` settings to processes added
afterwards.

With `--json`, `muster config` prints `{"settings": [{"key": "detach-key",
"value": "ctrl-\\"}, ...]}`, and `get` and `set` print one `{"key", "value"}`;
each value is written as `config set` reads it. The MCP server's `config_get`
answers in the same shapes. Its `config_set` changes only the `default-*`
settings; every other key, `mcp-stack-edits` among them, is changed from the
CLI alone.

## Commands

| Command | What it does |
|---|---|
| `muster ps [--all] [--commands]` | List processes with state, readiness, pid, uptime, restarts and usage; `--commands` adds each process's stored command |
| `muster up` | Start every autostart process |
| `muster down` | Stop every process |
| `muster start [name] [--wait [--timeout D]]` | Start a process, and with `--wait` wait until it is ready, or running when it has no readiness check |
| `muster stop [name]` | Stop a process through its stop policy |
| `muster restart [name] [--wait [--timeout D]]` | Restart a process, waiting as `start` does |
| `muster wait [name] [--log P [--regex] \| --port N \| --http URL [--status C] \| --state S] [--since CURSOR] [--timeout D]` | Wait until a process prints a line, accepts a connection, answers a URL, reaches a state, or passes its readiness check |
| `muster pause [name]` / `resume [name]` | Suspend and resume a process (SIGSTOP and SIGCONT) |
| `muster kill [name]` | Kill a process at once |
| `muster logs [name] [-f] [--screen] [-n N] [--since CURSOR] [--grep P [--regex]] [--errors]` | Print a process's log, follow it, print its current screen, or print filtered lines |
| `muster attach [name] [--read-only]` | Show a process in this terminal and type into it |
| `muster add <name> [options] -- <command>` | Add a process to the project, registering the folder |
| `muster set <name> [options]` | Change a process's fields |
| `muster rm <name>...` | Remove processes, stopping them first |
| `muster run [--name] -- <command>` | Add a command to the project, start it and attach |
| `muster config [list\|get\|set]` | Read and change the machine's settings |
| `muster export [FILE] [--force] [--with-env]` | Write the project's stack to a file |
| `muster import [FILE] [--merge\|--replace] [--dry-run]` | Load a stack file into the project |
| `muster init [--yes]` | Detect a stack in the folder and add it |
| `muster projects` / `projects rm <project>` | List registered projects, marking any whose stored processes do not read and listing by folder any whose own stored row does not, or remove one with its processes and coordination state; a project whose row does not read is removed by the folder text it is listed under, even one that is not an absolute path, and breaks no other project |
| `muster notices` | Show the stack edits agents made, and the failed starts, forced stops and crashes a running service recorded; never starts the service |
| `muster service status` / `stop` | Show or stop the service; processes keep running |
| `muster note` / `todo` / `kv` | Read and write the project's shared notes, todos and key-value state; `todo` shows which agent claimed a todo and for how long. `todo` and `kv` list a page of `--limit` entries (50 by default, at most 200) as `todo_list` and `kv_read` do: `todo --cursor <id>` reads on, and a `kv` listing cuts each value to 200 characters |
| `muster lock` / `lock release <name>` | List the locks agents hold on the project, or clear one an agent left behind |
| `muster coordination export\|import` | Move that shared state as YAML, the export readable by its owner alone; an import reads only a regular file of at most 16 MiB and refuses a key, note body, value, todo title or dependency list longer than a write may be |
| `muster mcp [--project P] [--as NAME] [--print-config]` | Serve muster to an agent over MCP |
| `muster doctor` | Diagnose the database, projects, stored processes and completions |
| `muster completions <shell>` | Print the shell line that enables completions |

A command that starts processes, and a wait for `running`, hold on until each
child has run for half a second, so one that exits at once is reported
crashed or exited rather than started.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | A process did not end up as asked, the action failed, or the current state refused it (a taken name, an import conflict, a folder holding a project, a todo another agent holds) |
| 2 | The command line could not be parsed, set changed nothing, a confirmation is missing, or a question was cancelled |
| 3 | The muster service is unavailable or runs a newer version |
| 4 | An invalid value: not a project, an unknown project or process, a refused argument, or an unusable file |
| 130 | Interrupted with Ctrl-C while it waited at a terminal |

Every invalid value exits 4, whether the command line parser refuses it
(`--timeout 0s`, an unknown setting key) or the command does (a bad regex, an
unknown project). Exit 2 is kept for a command line that does not parse at
all, such as an unknown flag or a missing argument. A valid request the
current state refuses exits 1 with the kind `conflict`: adding a name the
project already has, an import whose names collide with ones it keeps,
registering a folder that holds a registered project, or changing a todo
another agent holds without `--force`. A question is cancelled
with Esc or Ctrl-C.

### At a terminal

The terminal is interactive when stdin, stdout and stderr are all
terminals, `--json` is not given, `NO_COLOR` is unset or empty and `TERM` is
not `dumb`. There, the commands that act and `muster doctor` frame what they
do, with a spinner naming the processes still starting or stopping:

```text
┌  muster up · shop
│
├  /home/me/shop
│
●  web     running (pid 4123)
│
✗  worker  crashed: exited with status 1
│
└  1 ok · 1 failure
```

Ctrl-C closes the frame and exits with 130: before the request is sent it
cancels the command with nothing changed, and under the spinner the service
still finishes what it was asked. Data a command prints, such as a note, a
setting, a listing or the completions line, is never framed.

A command that needs a process name and was given none offers the project's
processes to choose from: `start`, `stop`, `restart`, `pause`, `resume`,
`kill` and `logs`, and `attach` offers the running ones. `muster projects rm`
asks before it removes a project.

Anywhere else muster prints one plain line per fact and asks nothing. A
question it might have to ask fails with exit code 4 and names the argument
that answers it, before it reaches the service, and `muster projects rm` needs
`--yes`.

### JSON

Every command but `mcp` and `completions` takes `--json`, the coordination
commands and `doctor` included, and prints results
and failures as JSON on stdout. `muster note`, `todo`, `kv` and `lock` answer
in the shapes the matching MCP tools do (`{"notes": [...]}`, a todo, `{"key",
"deleted"}`); `muster doctor --json` lists each `check` with its `outcome` and
says whether one `failed`, exiting 1 when one did.
Keys are snake_case, enum values are kebab-case (`on-failure`, `in-progress`),
durations are numbers of seconds and times are RFC 3339 in UTC to the second
(`"updated_at": "2026-10-29T15:34:41Z"`). A setting's `value` is the exception:
it is the text `config set` takes, such as `"5s"`. A process definition reads:

```json
{"name": "web", "command": "npm run dev", "working_dir": "frontend", "description": null,
 "environment": {"PORT": "5173"}, "autostart": true, "restart": "on-failure",
 "stop": {"signal": "terminate", "grace": 5.0}, "ready": {"port": 5173, "timeout": 30.0}}
```

A project is named as `{"name": "shop", "root": "/home/me/shop"}`, in the
output of every command and in each notice. `muster logs --json` names its
process under `process` and carries the `cursor` just past the text, to pass
as `--since`; `--screen` drops the blank rows below the text and has no
cursor. `muster logs -f --json` prints one object per line, each with the
`cursor` just past it.

## Attach

`muster attach <name>` takes over the current terminal like `tmux attach`: it
paints the process's screen at once, follows resizes and passes your keys
through. Without a name it offers the project's processes first. A one-line
hint, kept in the window title while attached, names the detach key: `ctrl-\`
by default (`muster config set detach-key`). Several clients may attach to one
process; `--read-only` watches without typing or resizing.

## Logs

Each process writes its raw output to a log under the state directory, across
restarts and after it exits. A log rotates at `log-max-size`, keeping one older
file. `muster logs` and `muster logs -f` write it byte for byte to a pipe or
file; on a terminal they keep text, line breaks, colours, cursor moves and
erases, so a progress bar redraws, and drop every query, terminal string and
other control, so a log cannot retitle the terminal, write its clipboard or
query it. When output rotates away twice before `muster logs -f` reads it,
the follow prints a line naming how many bytes were lost and carries on (with
`--json`, an object whose `skipped_bytes` counts them). `muster logs --screen`
prints what the terminal shows now.

Every byte of a process's output has an offset that runs on across restarts
and rotations, written as a cursor such as `o:18234`. `-n N`, `--since CURSOR`,
`--grep TEXT` (with `--regex`) and `--errors` print stripped lines instead: the
newest `N` (100 by default), only what follows the cursor, only matching lines,
or only error lines with their stack traces. With `--json` the result carries
the `cursor` to pass as `--since` next time, and `muster wait` reports the same
cursors. A wait, at the CLI or over MCP, reports how long it took as
`elapsed_s`, seconds to the millisecond (`2.14`), counted across every request
a long wait is made of; its text reads `after 2.140s`.

## Processes outlive the service

Every process runs under its own small holder process that owns its terminal.
Stopping, upgrading or crashing the service leaves every process running: the
next service finds the holders, reconnects and rebuilds each screen before it
answers, keeping each process's uptime, restart count, size and attached
clients, and carrying out any restart that was pending. A process removed from
its stack while the service was down is stopped when the service finds it. A
process whose holder dies while it keeps running, or whose live holder cannot
be reconnected to, is shown as `unsupervised` and never started a second time:
`stop` and `kill` signal it directly, and the service tries the holder again
on a backing-off timer or when you `start` or `restart` it, telling a failure
once per holder. A newer muster binary replaces an older running service in
place.

## MCP

`muster mcp --print-config` prints the snippet that adds muster to an agent's
MCP servers. It names no project, so one entry serves every project: each tool
call acts on the project its `project` argument names (a registered project's
name, or a folder), else on the deepest registered project containing the
folder the server runs in, or the one `--project` or `$MUSTER_PROJECT` names.
A folder inside a project acts on that project, and projects are found by the
same rule as the CLI's. Resolution runs per call, so a folder registered after the server started is found. Every result
about a project names it.

`--as NAME` sets the id the agent's coordination writes and locks carry; it
defaults to `MUSTER_AGENT`, then `herdr-pane-<HERDR_PANE_ID>`, then a random
session id, and is prefixed with the client's name (`claude-code@herdr-pane-3`).
`stack_status` and the first write result say which label is yours. A lock
belongs to the connection that took it: another connection under the same label
cannot renew or release it, and when the server exits cleanly it releases the
locks its own connection took.

An agent's loop is: `stack_status` to see what runs; after a code change,
`process_restart` with `wait: true`; when something breaks, `process_logs` with
`errors_only: true`, then again with `since` set to the returned cursor to read
only new output. The tools:

- the stack: `stack_status`, `stack_up` and `stack_events` (lifecycle events
  after a cursor, optionally long-polled); there is no `stack_down`, which is
  left to the person at the CLI;
- processes: `process_start`, `process_stop`, `process_restart`,
  `process_wait`, `process_logs`, `process_screen` (what the terminal shows
  now), `process_send`;
- the stack definition: `process_add`, `process_set` and `process_remove`, which
  edit it as `muster add`, `set` and `rm` do, within the guardrails below;
  `process_add` registers the folder when it is not yet a project, and `ready`
  stores a readiness check. It registers only the server's own folder or one
  inside it, under the same rule as the CLI, so a server started in `/` or your
  home folder must pass `project`;
- settings: `config_get`, and `config_set` for the `default-*` settings;
- coordination: `todo_list`, `todo_add`, `todo_claim`, `todo_update`,
  `todo_delete`, `lock_acquire`, `lock_release`, `note_read`, `note_write`,
  `note_delete`, `kv_read`, `kv_write`, `kv_delete`. A note body holds at most
  65536 characters, a value 4096, a todo title 500, a note or kv key 128 and
  a todo at most 32 dependencies; a longer one is refused over MCP, the CLI
  and `muster coordination import`. `todo_list`, `note_read` and `kv_read`
  page with `limit`, answering `more` and a `cursor` to pass for the next
  page, and a `kv_read` listing cuts each value to 200 characters and marks
  it `truncated`. Setting a todo `in-progress`
  claims it for the caller, and a forced change to another agent's todo takes
  its claim over.

An agent's stack edits are held to guardrails, since a changed command runs
with the user's rights at the next start:

- `muster config set mcp-stack-edits read-only` refuses every `process_add`,
  `process_set`, `process_remove`, `process_send` and `config_set` with an
  `isError` result naming the setting; `allow` lets them through again. Only
  the CLI changes it.
- Every edit an agent makes is recorded as a notice naming the agent, the
  process and the fields it actually changed, with the new command and working
  directory when it set them; a `process_set` that changes nothing records
  nothing. A `config_set` is recorded in every project, naming the setting and
  its new value; a `config_set` to the value already stored records nothing.
  It is kept in muster's database with the edit itself, in
  one transaction, the newest 50 per project, so it shows in `muster notices`,
  in `stack_status` and as a `notice` event in `stack_events` whether or not
  the service was running at the time.
- A `working_dir` must be a folder inside the project root; one outside it, or
  a symlink leading out of it, is refused. A stored one that climbs out of the
  root, as only a hand edit of the database can make, is refused when read,
  naming the process, and the process is never started.
- A command holds at most 8192 characters and a description 300, over MCP,
  the CLI and `muster import`.
- A definition an agent reads back lists its `environment` by variable name
  alone, never a value.

A wait takes one condition: `{"log": "Listening on"}` (add `"regex": true` for a
regex), `{"port": 3000}`, `{"http": "http://localhost:3000/health"}` (with an
optional `status`), `{"state": "running"}` or `{"ready": true}`. A `port` or
`http` condition only reaches loopback hosts. A stored readiness check takes
the same shape with a `timeout` in seconds, such as `{"log": "Listening on",
"timeout": 60}`; `process_set` with `ready: null` removes it. Durations go in
as they come out, as numbers of seconds: `stop_grace` is `5`, never `"5s"`,
and text is refused naming seconds.

Cursors are offsets in a process's output that run on across restarts, so
`process_logs` with `since` set to a wait's `cursor`, or to `run_start` from
`stack_status`, returns only what followed. `stack_status` also lists each
process's readiness, the ports its process tree listens on, its URLs, how it
last exited and why, and `blocked_on` when its start was refused because
another process holds its readiness port.

Event cursors, such as `e:3141592:9921`, name the service that numbered them.
`stack_events` with a cursor from an earlier service, after a restart, or past
the newest event answers at once with `gap: true` and every event held, never
waiting; so does one whose events were dropped from the 1024 kept. A gap means
re-reading `stack_status`. An agent edit one service raised as an event is
never raised again by the next.

No call blocks longer than 45 seconds; a wait that has not finished answers with
status `waiting` and a cursor to continue from, and a request carrying a
progress token hears progress every 5 seconds. A value outside its bound is
refused, never cut to fit: a `timeout_s` or `wait_s` past 45, a `limit` of 0 or
past 200, and a wait whose `since` lies past the output written so far. Output is stripped of terminal
escapes and capped, saying what it dropped. Every stack and process result
carries `alerts`: crashes, readiness timeouts and blocked starts since the
agent's last call, so it learns of a crash without polling. Each is reported
once, even to calls made at the same time, and `alerts_gap: true` says events
were dropped before they were read, so some alerts were missed. `stack_events`
carries none and never marks an alert heard.

A mistake in a call, such as an unknown name, a malformed cursor or an invalid
value, comes back as a result flagged `isError` whose text says what to do and
whose `error` names its `kind` and up to 20 `candidates`. So does a wait ended
by an exit or a crash, with the run's last lines. A wait that ran out of time,
and a lock or todo someone else holds, are not errors.

## Using it with herdr

Agents and interactive terminals run in [herdr](https://herdr.dev) panes, each
agent with muster's MCP server, so it can read and drive the stack its human
sees in `muster ps`. A herdr custom command can also open a process in a pane
of its own:

```toml
[[keys.command]]
key = "prefix+w"
type = "pane"
command = "muster attach web"
```

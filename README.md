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
starts it in the background.

## Projects and stacks

A project is a folder. The first `muster add`, `muster run`, `muster import` or
`muster init` that adds a process to a folder registers it, named after the
folder (`shop`, then `shop-2` for a second folder of that name). Its stack is
the list of processes it defines, in the order they were added.

Everything muster stores lives in one SQLite database, `muster.db` in muster's
state directory (`~/.local/state/muster` on Linux,
`~/Library/Application Support/muster` on macOS), readable by its owner only:
every project's stack, the machine's settings and the shared notes, todos and
key-values. The CLI and the MCP server write it directly and tell a running
service what changed, so an edit reaches the service at once and works without
it.

Commands act on the project given by `--project` (a registered name, or a
folder when the value contains `/` or starts with `.` or `~`), then
`$MUSTER_PROJECT` (set inside every process muster runs), then the deepest
registered folder containing the current directory. Bare `muster` is
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

- a name, unique within the project and free of control characters, that
  every command addresses it by;
- a command, run by the shell;
- `--cwd`, where it runs, stored relative to the project folder when inside it;
  the project folder when left out;
- `--env KEY=VALUE`, repeatable, the variables it runs with;
- `--description`, a one-line note;
- `--autostart` or `--no-autostart`, whether `muster up` starts it;
- `--restart`: `never`, `on-failure` or `always`; restarts back off;
- `--stop-signal`: `terminate` (SIGTERM) or `interrupt` (SIGINT), and
  `--stop-grace`, how long a stop waits before SIGKILL.

A field left out takes the matching `default-*` setting, and the process stores
that value, so changing a default later changes only processes added after it.
`muster add` never starts anything.

`muster set <name>` changes the fields it is given: `--command` or a new command
after `--`, `--no-cwd` and `--no-description` to clear, `--env` to add or
replace a variable and `--unset-env` to remove one. A running process keeps its
current run; `muster restart <name>` applies the change. `muster rm <name>...`
stops each process it removes and waits for it to end. A name cannot be
changed: remove the process and add it again.

### Sharing a stack

`muster export [FILE]` writes the project's stack to `muster.yml` in the project
folder, or to `FILE` (`-` for stdout), and refuses to overwrite a file without
`--force`. Environment variables are written by name with a null placeholder;
`--with-env` writes their values too. `muster import [FILE]` loads such a file
into the current folder, registering it when needed. By default it refuses
names the project already defines; `--merge` replaces those, `--replace` makes
the stack exactly the file and stops what it removes, and `--dry-run` prints
the plan. An import refuses a file whose environment still holds a placeholder.

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
```

[examples/muster.yml](examples/muster.yml) is a complete example. The file
never names the project: the folder that imports it is the project.

### Detecting a stack

`muster init` reads `package.json` scripts, `Procfile`, `justfile`, `Makefile`
and compose files in the folder and offers what it finds, with the likely dev
commands preselected. It only reads files and never runs them. At a terminal it
asks which to add; elsewhere it prints the candidates, and `muster init --yes`
adds the suggested ones.

## Settings

`muster config` lists the machine's settings, `muster config get <key>` prints
one and `muster config set <key> <value>` changes it.

| Key | Value |
|---|---|
| `desktop-notifications` | `true` or `false`: whether failures raise a desktop notification |
| `default-size` | `ROWSxCOLS`, such as `24x80`: the terminal size a process starts with, 1 to 1000 rows by 2 to 2000 columns |
| `detach-key` | a Control chord, such as `ctrl-\`: the key that ends `muster attach`; `ctrl-m`, `ctrl-j`, `ctrl-i`, `ctrl-h` and `ctrl-c` are refused, since terminals send them for Enter, Tab, Backspace and interrupt |
| `log-max-size` | a size, such as `10MiB`: when a process's log rotates |
| `scrollback` | lines of history each process's terminal keeps |
| `color-scheme` | `dark` or `light`: the default colors a process's terminal reports |
| `default-autostart` | `true` or `false` for a new process |
| `default-restart` | `never`, `on-failure` or `always` for a new process |
| `default-stop-signal` | `terminate` or `interrupt` for a new process |
| `default-stop-grace` | a duration, such as `5s`, for a new process |

`detach-key` applies at the next attach, `desktop-notifications` at once, the
size, scrollback, log size and color scheme to processes started afterwards, and
the `default-*` settings to processes added afterwards.

With `--json`, `muster config` prints `{"settings": [{"key": "detach-key",
"value": "ctrl-\\"}, ...]}`, and `get` and `set` print one `{"key", "value"}`;
each value is written as `config set` reads it. The MCP server's `config_get`
answers in the same shapes.

## Commands

| Command | What it does |
|---|---|
| `muster ps [--all]` | List processes with state, pid, uptime, restarts and usage |
| `muster up` | Start every autostart process |
| `muster down` | Stop every process |
| `muster start [name]` | Start a process |
| `muster stop [name]` | Stop a process through its stop policy |
| `muster restart [name]` | Restart a process |
| `muster pause [name]` / `resume [name]` | Suspend and resume a process (SIGSTOP and SIGCONT) |
| `muster kill [name]` | Kill a process at once |
| `muster logs [name] [-f] [--screen]` | Print a process's log, follow it, or print its current screen |
| `muster attach [name] [--read-only]` | Show a process in this terminal and type into it |
| `muster add <name> [options] -- <command>` | Add a process to the project, registering the folder |
| `muster set <name> [options]` | Change a process's fields |
| `muster rm <name>...` | Remove processes, stopping them first |
| `muster run [--name] -- <command>` | Add a command to the project, start it and attach |
| `muster config [list\|get\|set]` | Read and change the machine's settings |
| `muster export [FILE] [--force] [--with-env]` | Write the project's stack to a file |
| `muster import [FILE] [--merge\|--replace] [--dry-run]` | Load a stack file into the project |
| `muster init [--yes]` | Detect a stack in the folder and add it |
| `muster projects` / `projects rm <project>` | List registered projects, or remove one with its processes and coordination state |
| `muster notices` | Show failed starts, forced stops and crashes the service recorded |
| `muster service status` / `stop` | Show or stop the service; processes keep running |
| `muster note` / `todo` / `kv` | Read and write the project's shared notes, todos and key-value state; `todo` shows which agent claimed a todo and for how long |
| `muster lock` / `lock release <name>` | List the locks agents hold on the project, or clear one an agent left behind |
| `muster coordination export\|import` | Move that shared state as YAML |
| `muster mcp [--print-config]` | Serve the project to an agent over MCP |
| `muster doctor` | Diagnose the database, projects and completions |
| `muster completions <shell>` | Print the shell line that enables completions |

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | A process did not end up as asked, or the action failed |
| 2 | The command line could not be parsed, `set` changed nothing, a confirmation is missing, or a question was cancelled with Esc or Ctrl-C |
| 3 | The service is unavailable or runs a newer version |
| 4 | Not a project, an unknown project or process, a taken name, an invalid value, an import conflict, or an unusable stack file |
| 130 | Interrupted with Ctrl-C while it waited at a terminal |

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

The commands take `--json` and print results and failures as JSON on stdout.
Keys are snake_case, enum values are kebab-case (`on-failure`, `in-progress`),
durations are seconds and times are RFC 3339 in UTC. A process definition
reads:

```json
{"name": "web", "command": "npm run dev", "working_dir": "frontend", "description": null,
 "environment": {"PORT": "5173"}, "autostart": true, "restart": "on-failure",
 "stop": {"signal": "terminate", "grace": 5.0}}
```

`muster logs -f --json` prints one object per line.

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
file. `muster logs` reads it; `muster logs --screen` prints what the terminal
shows now.

## Processes outlive the service

Every process runs under its own small holder process that owns its terminal.
Stopping, upgrading or crashing the service leaves every process running: the
next service finds the holders, reconnects and rebuilds each screen before it
answers, keeping each process's uptime, restart count, size and attached
clients, and carrying out any restart that was pending. A process removed from
its stack while the service was down is stopped when the service finds it. A
process whose holder dies while it keeps running is shown as `unsupervised` and
never started a second time. A newer muster binary replaces an older running
service in place.

## MCP

`muster mcp --print-config` prints the snippet that adds muster to an agent's
MCP servers for the current project. The server exposes the project to the
agent:

- the stack: `process_add`, `process_set` and `process_remove`, which edit it
  exactly as `muster add`, `set` and `rm` do; `process_add` registers the folder
  when it is not yet a project;
- live processes: `process_list`, `process_output`, `process_start`,
  `process_stop`, `process_restart`, `process_send`;
- settings: `config_get`, `config_set`;
- shared notes: `scratchpad_list`, `scratchpad_get`, `scratchpad_set`,
  `scratchpad_delete`;
- shared todos: `todo_list`, `todo_get`, `todo_add`, `todo_set_status`,
  `todo_delete`;
- shared key-value state: `kv_list`, `kv_get`, `kv_set`, `kv_delete`.

A mistake in a call, such as an unknown name or an invalid value, comes back as
a result flagged `isError` whose text names the field or the valid names.

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

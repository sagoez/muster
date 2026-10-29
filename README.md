# muster

The supervisor and control plane behind your terminal.

muster owns what runs in a project: its dev servers, watchers, shells and CLI
agents. A per-user background service starts them, restarts them, keeps their
logs and screens, and answers a CLI and an MCP server. Your terminal stays your
choice: muster draws no panes, and `muster attach` shows any process in the
terminal you are already in.

**Status: Beta.** Linux and macOS.

![muster demo](docs/muster-demo.gif)

## Install

Requires a recent Rust toolchain.

```sh
cargo install muster-workspace
muster init           # write a muster.yml here and register the folder
muster up             # start its autostart processes
muster hooks setup    # let agent CLIs report their session ids, for resume
```

There is no daemon to start. The first `muster` command that needs the service
starts it in the background.

## muster.yml

A project is a `muster.yml` with three sections, `agents`, `terminals` and
`commands`. Every section is present; use `[]` for an empty one.

```yaml
agents:
  - name: claude
    command: claude
    autostart: false

terminals:
  - name: shell
    command: null          # null runs your login shell
    autostart: true

commands:
  - name: web
    command: npm run dev
    description: the dev server
    autostart: true
    restart: on-failure
    stop:
      signal: interrupt
      grace_period: 10s
```

Each process takes:

- `name`: unique within the project, and how every command addresses it.
- `autostart`: required. `true` starts the process with `muster up`; `false`
  leaves it for `muster start`.
- `command`: the shell command. `null` on a terminal runs your login shell.
- `working_dir`: where it runs, relative to the project folder. Defaults to the
  project folder.
- `description`: an optional one-line note.
- `restart`: `never` (the default), `on-failure` or `always`. Restarts back off.
- `stop`: commands only. `signal` is `terminate` (SIGTERM) or `interrupt`
  (SIGINT), and `grace_period` is how long muster waits before SIGKILL. The
  default is `terminate` with `5s`.

muster reconciles edits to `muster.yml` live: new entries appear, removed ones
retire, and a running process whose entry is unchanged keeps running.

Machine-wide settings live in `settings.yml` in muster's config directory
(`~/.config/muster` on Linux, `~/Library/Application Support/muster` on
macOS), written with explicit defaults on first run:

- `desktop_notifications`: whether failures raise a desktop notification.
- `default_size`: the terminal size a process starts with, 1 to 1000 rows by
  2 to 2000 columns. An attached terminal outside that range is clamped to it.
- `detach_key`: the key that ends `muster attach`.
- `log_max_size`: the size at which a process's log rotates.
- `scrollback`: the lines of history each process's terminal keeps.
- `color_scheme`: `dark` or `light`, the default colors a process's terminal
  reports when an agent asks for them, so it themes for your terminal.

## Commands

Process commands act on the project of `--config`, then `$MUSTER_PROJECT` (set
inside every process muster runs), then `./muster.yml`, then the first
registered project. Bare `muster` is `muster ps`.

| Command | What it does |
|---|---|
| `muster ps [--all]` | List processes with state, pid, uptime, restarts and usage |
| `muster up` | Start every `autostart: true` process |
| `muster down [--agents [--yes]]` | Stop commands and terminals, and agents with `--agents` |
| `muster start [name]` | Start a process |
| `muster stop [name]` | Stop a process through its stop policy |
| `muster restart [name]` | Restart a process |
| `muster pause [name]` / `resume [name]` | Suspend and resume a process (SIGSTOP and SIGCONT) |
| `muster kill [name]` | Kill a process at once |
| `muster logs [name] [-f] [--screen]` | Print a process's log, follow it, or print its current screen |
| `muster attach [name] [--read-only]` | Show a process in this terminal and type into it |
| `muster run [--name] [--kind] -- <command>` | Add a command to the project, start it and attach |
| `muster agent new <tool> [--command] [--resume]` / `close <name>` / `reopen` | Open, close and reopen agent sessions |
| `muster notices` | Show failed starts, forced stops and crashes the service recorded |
| `muster service status` / `stop` | Show or stop the service; processes keep running |
| `muster init` | Write a `muster.yml`, asking for its processes, and register the folder |
| `muster check` | Validate a `muster.yml` |
| `muster projects [add\|remove]` | List and manage registered projects |
| `muster note` / `todo` / `kv` | Read and write the project's shared notes, todos and key-value state |
| `muster coordination export\|import` | Move that shared state as YAML |
| `muster mcp [--print-config]` | Serve the project to an agent over MCP |
| `muster hooks setup` | Install the agent session hooks |
| `muster doctor` | Diagnose config, projects, sessions, hooks and completions |
| `muster completions <shell>` | Print the shell line that enables completions |

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | A process did not end up as asked, or the action failed |
| 2 | The command line could not be parsed, or a question was cancelled with Esc or Ctrl-C |
| 3 | The service is unavailable or runs a newer version |
| 4 | An unknown project or process name, or an invalid config |
| 130 | Interrupted with Ctrl-C while it waited at a terminal |

### At a terminal

The terminal is interactive when stdin, stdout and stderr are all
terminals, `--json` is not given, `NO_COLOR` is unset or empty and `TERM` is
not `dumb`. There, the commands that act and `muster doctor` frame what they
do, with a spinner naming the processes still starting or stopping:

```text
┌  muster up · shop
│
├  /home/me/shop/muster.yml
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
listing or the completions line, is never framed.

A command that needs a process name and was given none offers the project's
processes to choose from: `start`, `stop`, `restart`, `pause`, `resume`,
`kill` and `logs`, and `attach` offers the running ones. While an agent
runs, `muster down --agents` asks before it stops the agents; `--yes`
answers for it, and answering no stops nothing and exits with 0. `muster
init` asks for each process's name, kind, command, autostart and restart
policy, then writes them to `muster.yml`.

Anywhere else muster prints one plain line per fact and asks nothing. A
question it might have to ask fails with exit code 4 and names the argument
that answers it, before it reaches the service: a missing name, and
`down --agents` without `--yes` whether or not an agent runs. `muster init`
writes a starter `muster.yml` with one terminal.

### JSON

`ps`, `up`, `down`, `start`, `stop`, `restart`, `pause`, `resume`, `kill`,
`logs`, `attach`, `run`, `agent new`, `agent close`, `agent reopen`,
`notices`, `service status` and `service stop` take `--json` and print
results and failures as JSON on stdout. Keys are snake_case, enum values are
kebab-case (`on-failure`, `in-progress`), durations are whole seconds and
times are RFC 3339 in UTC. `muster logs -f --json` prints one object per
line.

## Attach

`muster attach <name>` takes over the current terminal like `tmux attach`: it
paints the process's screen at once, follows resizes and passes your keys
through. Without a name it offers the project's processes first. A one-line
hint, kept in the window title while attached, names the detach key: `ctrl-\`
by default (`detach_key` in `settings.yml`). Several clients may attach to one
process; `--read-only` watches without typing or resizing.

## Logs

Each process writes its raw output to a log under the state directory
(`~/.local/state/muster/logs` on Linux,
`~/Library/Application Support/muster/logs` on macOS), across restarts and
after it exits. A log rotates at `log_max_size`, keeping one older file. The
state directory, its logs and the coordination database are readable by their
owner only.
`muster logs` reads it; `muster logs --screen` prints what the terminal shows
now.

## Processes outlive the service

Every process runs under its own small holder process that owns its terminal.
Stopping, upgrading or crashing the service leaves every process running: the
next service finds the holders, reconnects and rebuilds each screen before it
answers, keeping each process's uptime, restart count, size and attached
clients, and carrying out any restart that was pending. A process whose holder
dies while it keeps running is shown as `unsupervised` and never started a
second time. A newer muster binary replaces an older running service in place.

## Agents

`muster agent new claude` opens an agent session in the project; `muster agent
close <name>` moves it into history and `muster agent reopen` resumes the latest
closed conversation. Agents listed under `agents:` in `muster.yml` resume their
own conversation when they start. Run `muster hooks setup` once so Claude,
Codex, Gemini, Amp, OpenCode, Copilot and Kimi report their session ids, and
again after upgrading muster: `muster doctor` reports hooks an older muster
installed, and a capture from one asks for the same.

Any other agent runs as `custom` with its own command:

```sh
muster agent new custom --name scout --command mycodex \
  --resume 'mycodex resume {session_id}'
```

`--resume` is optional. In it, `{session_id}` stands for the agent's
conversation id and must be a whole unquoted word; without it muster appends
the id to the template. muster refuses a template whose placeholder is quoted
or embedded in another word. Such an agent reports its id through the
[agent protocol](docs/agent-protocol.md).

## MCP

`muster mcp --print-config` prints the snippet that adds muster to an agent's
MCP servers. The server exposes the project to the agent:

- processes: `process_list`, `process_output`, `process_start`, `process_stop`,
  `process_restart`, `process_send`, `session_list`;
- shared notes: `scratchpad_list`, `scratchpad_get`, `scratchpad_set`,
  `scratchpad_delete`;
- shared todos: `todo_list`, `todo_get`, `todo_add`, `todo_set_status`,
  `todo_delete`;
- shared key-value state: `kv_list`, `kv_get`, `kv_set`, `kv_delete`.

## Using it with herdr

muster works in any terminal. In [herdr](https://herdr.dev), a custom command
can open a process in a pane of its own:

```toml
[[keys.command]]
key = "prefix+w"
type = "pane"
command = "muster attach web"
```

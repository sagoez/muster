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

A project needs no process. `muster projects add [FOLDER]` registers a folder,
the current one by default, with nothing to run, so notes, todos, values and
locks work there; agents do the same with the MCP tool `project_register`. It
refuses what `add` refuses, and a folder inside a project too, whose project
covers it already. Its folder is only ever the argument: `--project` names a
project, so `projects add` refuses it. A folder that is a project already is
reported as one and the command succeeds, writing nothing. Notes, todos,
values, locks and `coordination import` never register a folder: outside every
project they fail and name `muster projects add`.

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
the agent edit log and how each process last ended. The state directory also
keeps holder records and process logs. No new holder puts its socket there.

Each process's holder listens on a Unix socket in `/tmp/muster-<uid>/<key>/`,
a folder named for the state directory. The path is short enough to fit a
Unix socket address however long your home folder is, and no logout removes
it. Both folders must be yours with mode 0700, and `muster doctor` reports one
that is not. The error names the folder, its owner and what clears it: one of
your own with another mode only needs `chmod 700`; a link to a folder is
replaced by that folder, so nothing kept in it is lost; any other file or link
in its place is removed; one another user made in `/tmp` stays until its owner
or an administrator removes it, or a reboot clears `/tmp`, and no holder can
start until then. Every path in a suggested command is quoted for the shell.

A holder binds its socket once. If something deletes a running holder's
socket, by hand or a tmp cleaner, that holder can never be reached again. A
service already connected keeps working with it and still collects its exit,
but `muster attach` cannot reach it, and a service started later shows the
process as running unsupervised and stops it by signal. The holder itself
then never exits on its own: it stays, waiting for a service that can no
longer reach it, until you kill it or reboot. `ps ax -o pid,command | grep
__holder` lists every holder with the spec it started from, which is named
like its socket with `.json` in place of `.sock`. Keep tmp cleaners away from
`/tmp/muster-<uid>`.

Every muster process must see the same `/tmp`: a service or terminal run in a
sandbox with a private `/tmp` (systemd `PrivateTmp=`, some Flatpak or snap
apps) cannot reach the holders of one that is not.

A holder an earlier muster started keeps its socket in `holders/sock` in the
state directory. The service still adopts it there. At each start it removes
the handover files in that folder and the sockets of holders that ended, and
it removes the folder once it is empty. `muster doctor` says when the folder
holds only running holders' sockets, which is fine, and warns about anything
else left there, saying whether the next start removes it or you should.

The service listens in `$XDG_RUNTIME_DIR/muster`, else `/tmp/muster-<uid>`,
which must be yours with mode 0700, and records its socket in
`service.socket` in the state directory so a command started with a different
runtime directory still reaches it. A client talks only to a service running
as you. A refused service socket folder's error says what clears it, as above.
When another user made `$XDG_RUNTIME_DIR/muster`, you can also set
`XDG_RUNTIME_DIR` to a directory of your own to move the service socket;
that does not help with `/tmp/muster-<uid>`, which holds every process's
socket too. A service socket path too long to fit is refused with a hint to
set a shorter `XDG_RUNTIME_DIR`.

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
- `--icon NAME`, the mark `muster view` draws it with, a brand name such as
  `vite` or `postgresql`; a wrong name lists the known ones. Left out, none
  is stored and `muster view` shows one picked from its command;
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

`muster set <name>` changes only what it is given: `--command` or a new
command after `--`, `--env` and `--unset-env`, `--icon NAME`, and
`--no-cwd`, `--no-description`, `--no-icon`, `--no-ready` and `--no-ports`
to clear. `--port` replaces the stored set. A running process keeps its
current run until `muster restart`, and `set` says when one is needed. A
`set` that changes nothing succeeds and says so (`"changed": false`).
`muster rm` stops each process it removes and waits for it. A name cannot
be changed; remove and add again.

A command that starts processes, and a wait for `running`, holds on until each
child has run for half a second, so one that exits at once is reported failed
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
    icon: vite
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

`icon` is optional, one of the names `muster add --icon` accepts, and a name
muster does not know is refused on import. `ready` is optional and takes one of
`log` (with `regex: true`), `port` or `http`, plus a `timeout`. `ports` is
optional, ascending, and left out when empty. The file never names the
project: the folder that imports it is the project.
[examples/muster.yml](../examples/muster.yml) is a complete example.

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
| `muster ps [--all] [--commands] [--wide]` | List processes, one line each with its state, ports, uptime, usage, restarts and last exit, why a failed one failed, a count by state and what to run next; the heading shows the shells in the project or the time left before it stops. `--wide` prints every column as a table, readiness and pid included |
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
| `muster view [--port N] [--no-open]` | Serve a live, read-only page of every project's processes on 127.0.0.1 and open it in the browser, on the floor `-p` names; never starts the service |
| `muster config [list\|get\|set]` | Read and change settings |
| `muster export [FILE] [--force] [--with-env]` | Write the stack to a file |
| `muster import [FILE] [--merge\|--replace] [--dry-run]` | Load a stack file |
| `muster init [--yes]` | Detect a stack in the folder and add it |
| `muster shell-init <bash\|zsh\|fish>` | Print the shell hook |
| `muster projects` / `projects rm <project>` | List registered projects, or remove one with its processes and coordination state (the folder is untouched) |
| `muster projects add [FOLDER]` | Register a folder, the current one by default, as a project with no process |
| `muster projects set [PROJECT] --idle-stop <DURATION\|never\|default>` | Give a project its own idle stop, or follow the setting again |
| `muster notices` | Show the stack edits agents made, and the failed starts, forced stops and crashes the service recorded; never starts the service |
| `muster service status` / `stop` | Show or stop the service; processes keep running |
| `muster note` / `todo` / `kv` | Read and write the project's shared notes, todos and key-value state, paged with `--limit` (50 by default, at most 200) and `--cursor` |
| `muster lock` / `lock release <name>` | List agents' locks, or clear one left behind |
| `muster coordination export\|import` | Move the coordination state as YAML |
| `muster mcp [--project P] [--as NAME] [--print-config]` | Serve muster to an agent over MCP |
| `muster mcp add\|remove <claude\|codex\|zed>` | Install muster's plugin, its MCP server and session-start hook, through the agent's own plugin commands, or uninstall it; for Zed, print the server entry |
| `muster mcp context [--project P]` | Print the project's notes and open todos for an agent's session; nothing outside a project, always exit 0 |
| `muster doctor` | Check the database, projects, stored processes, completions, the shell hook and the terminal workspace |
| `muster completions <shell>` | Print the line that enables completions |

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | A process did not end up as asked, the action failed, or the current state refused it (a taken name, an import conflict, a folder holding a project, a folder `projects add` finds inside a project, a todo another agent holds) |
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

Every line that says how something went carries a mark:

| Mark | Colour | Means |
|---|---|---|
| `◇` | green | done or healthy: an action that worked, a process running and ready |
| `▲` | yellow | a warning, the command goes on: starting, not ready, restarting, unsupervised |
| `■` | red | a problem: failed, or the command stopped |
| `○` | dim | idle: stopped, pending, paused |

Colour means status only; detail sits two spaces under its mark, dimmed. A
stream is coloured only when it is a terminal and `NO_COLOR` is unset or empty;
`CLICOLOR` and `CLICOLOR_FORCE` are honoured. Lines marked `▲` or `■`, with
their detail, go to stderr and every other line to stdout. Listings are the
exception: `ps`, `notices`, `projects` and the other commands that answer with
data print every line on stdout, so `muster ps | grep` still sees a failed
process. A process whose child ended with a failure reads `failed` (its
`--json` state stays `crashed`), and an exit reads the same everywhere:
`exit 127`, `signal SIGKILL`. `muster wait` marks a match `◇`, a wait that ran
out of time `▲` with how to keep waiting, and a failure `■` with the run's last
lines. A command that can fail ends by saying what to run next, as a command
that works when pasted: it names the project with `-p` when the command was
run with `-p`, under `--all` or outside the project's folder, by its folder
written from `~` when the name starts with `.` or `~` or holds a `/`, which
`-p` would take for a folder where the command is pasted, passes a name
that starts with `-` after `--`, and offers `muster up` only when a stopped
process autostarts, else `muster start` for the first:

```
starter  ~/Work/starter

■ infra      failed    exit 1 · 2m ago
  Conflict. The container name "/pubsub-emulator" is already in use by…
◇ web        running   :3000 · up 2h 4m · 1.2% · 128.0 MiB · attached
▲ worker     starting  not ready yet · 2 restarts
○ analytics  pending

1 failed · 1 running · 1 starting · 1 pending
Run muster logs infra to see why it failed.
```

The reason under a failed process is the process's own text, not muster's: the
last line with text that its failed run logged, read as `muster logs -n`
reads it from at most the run's newest 64 KiB, made printable, its tabs
expanded, and cut to the width of the terminal it is drawn on. A run that
never launched shows the error that kept it from launching instead, and its
next step is `muster notices`, since it logged nothing; one that logged
nothing shows no reason. A process stopped while its policy waited to
restart it reads `stopped`, in `ps`, `ps --wide` and the chooser alike. The
service does not store a stop, so after the service restarts, a process
stopped on request reads `pending`, never `failed`.

A command that fails exits 1 even when the reader of its output went away,
as `| head -1` leaves it: the code is set before anything is written.

## JSON

Every command but `mcp`, `completions` and `shell-init` takes `--json` and
prints results and failures as JSON on stdout. Keys are snake_case, enum
values kebab-case (`on-failure`, `in-progress`), durations are seconds (an
`idle_stop` that never comes is `null`), and times are RFC 3339 in UTC to
the second. A setting's `value` is the text `config set` takes, such as
`"5s"`. A process definition:

```json
{"name": "web", "command": "npm run dev", "working_dir": "frontend", "description": null,
 "icon": "vite",
 "environment": {"PORT": "5173"}, "autostart": true, "restart": "on-failure",
 "stop": {"signal": "terminate", "grace": 5.0}, "ready": {"port": 5173, "timeout": 30.0},
 "ports": [5173]}
```

A project appears as `{"name": "shop", "root": "/home/me/shop"}`. A command or
tool that may register a project (`add`, `run`, `import`, `init --yes`,
`projects add`, `process_add`, `project_register`) names it the same way under
`registered`, present only when that call registered it. `muster logs --json` carries the
`cursor` just past the text, to pass as `--since`;
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
count as entering. The hook passes the time of the first prompt it drew in its
folder, and a project registered later was registered around the shell, even
when the report of the `cd` reads the projects only after the registration.
bash 5 and zsh read that time from their own clock and fish from `date`; a
shell that cannot read it keeps the time of its first report in that folder.
A muster command that registers a project, run straight from a hooked shell
with both inside the new project, as
`cd x && muster add --autostart web -- npm run dev` is, also tells that
shell's record it is there, so the prompt after it is not an entering either;
once that shell has left, coming back is an ordinary entering. A registration
from elsewhere, as `muster -p x add`, or run by anything but the shell
itself, as a child shell, a script or `npm` is, gives the shell no such
pass. A process you stopped yourself stays stopped.

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

## View

`muster view` serves a live, read-only page of every project's processes and
runs in the foreground until Ctrl-C. It listens on `127.0.0.1` only, on
`--port` or on a free port, and prints its address as the first line, then
`Press Ctrl-C to stop.`. Unless `--no-open` is given it opens the page with
`$BROWSER`, taken as one program path with no arguments of its own. Since
the user chose it, nothing else is tried when it is not there, cannot run
or exits unsuccessfully within a second. With `$BROWSER` unset or empty, it
opens the page with the first of `xdg-open`, `wslview`, `sensible-browser`
and `x-www-browser` that works (`open` on macOS). An opener that is not
there, a bare name that `PATH` finds only where it may not run, and one
that exits unsuccessfully within a second, as `xdg-open` does with no
handler for the page, each move on to the next; one still running after
that second counts as started. When no browser opens, the view says why
and leaves the address to open by hand. A browser it starts runs in a process group of its
own, so the Ctrl-C or hangup that ends the view never reaches it. It never
starts or replaces the service and never loads a project into it.
`-p <project>`, or the `MUSTER_PROJECT` muster gives its processes, opens
the page on that project's floor, resolved as every command resolves it, as
the address's `#/<project>` fragment; without either the page opens on the
project holding the current folder. A project that is not registered is
refused before
anything listens, and so is any project while the database an older muster
wrote still waits for its migration, the refusal naming the command that
applies it.

The server accepts before the view reads anything, so a page that opens
first shows the floor loading until the first readings arrive, its
processes held back until the first status. Every reading runs on a thread
of its own: the projects every 5 seconds, the notes every 2, the status
every second or at once after an event, the service's events held open up
to 30 seconds, and each running process's log statted every second. A pin
drops on a process's face only for a service event, once each, as it is
heard: ok for a start the service says armed no readiness check or for a
passed check, again for a restart, fail for a crash or a refused start, late
for a check out of time. Events from before the page's latest fresh snapshot
pin nothing, and one heard while a fresh snapshot is due pins when that
snapshot arrives, if it is after the snapshot's cursor. While
no page streams, every reading but the projects' parks, and the projects
look at a stack's manifests for its icons only when the stack changed.
The first page to connect after that, or to the
view at all, gets a snapshot of the floor as last read, loading and with
every process drawn unconfirmed, then a fresh snapshot once the status
reading it woke arrives. The database is opened read-only and opened again
when its file is replaced, when no project's stack reads, or when one
fails for anything but a row of its own; an older database shows a banner
asking to run any muster command, which migrates it. A project whose stack
does not read is a floor of its own marked unreadable, keeping the
processes it last read, drawn unconfirmed, while the other floors read on.

While the service is absent, starting or of another version, and after 5
failed asks in a row of a service that holds its socket, the page says so
in a banner and every process keeps its room, its mark and its card's
command, drawn dimmed as unconfirmed, since its child runs on under its
holder. The card of an unconfirmed process says why: the service is not
reporting, the readings park while no page watches and the floor then waits for a fresh status, or its
project's stack does not read. A card's command names its project with
`-p <name>`, or with `-p <folder>` written from `~` when the name starts
with `.` or `~` or holds a `/`, which `-p` would take for a folder under
wherever the command is pasted.

The address carries a random token in its path,
`http://127.0.0.1:<port>/v/<token>/`, fresh each run, and the whole page
lives under that path: the shell, `assets/v<version>/` (so a browser never
runs a cached file of another muster), `events` and
`api/log?project=<project>&name=<process>`. Any other path is 403, so other
local users and other browser tabs get nothing. No cookie is used, since a
browser sends a `127.0.0.1` cookie to every port, the dev servers muster
runs among them. To keep the token out of process arguments, which every
local user can read, the browser is opened at
`http://127.0.0.1:<port>/open/<code>` instead: a one-time code, held only in
memory, minted once the server serves, that redirects to the address once.
Each opener tried gets a fresh code just before it starts, which voids the
one before, so an opener that fails never spends the next one's time. A
code is good for 10 seconds and used up by the first request that names it;
a wrong code is 403 and leaves it pending, and after its use `/open/...` is
403.

`events` is a Server-Sent Events stream that opens with `retry: 1000` and a
snapshot, then sends each change as it happens. After 5 seconds of silence
it looks whether the page hung up and otherwise sends a keepalive comment,
and a busy stream learns it from its next writes failing, so a closed tab
gives its connection back within seconds.
At most 16 streams are open at once, and the next is 503; a page that falls
256 messages behind is dropped and catches up with a snapshot when it
reconnects. A browser opens at most 6 connections to one host and each tab's
stream holds one, so with 6 tabs of the view open a card's log read, and a
seventh tab, wait until one closes; close the tabs you do not use.

The log route answers `{"lines": [...]}` with the last 12 lines that show
anything, read from at most the last 16 KiB of the log, without color and
made printable. A process the floor does not hold is a 404 with
`{"error": "unknown-process"}`, and a log that cannot be read a 500 with
`{"error": "unreadable-log"}`. Once the view's floor thread has ended,
`events` and the log route are 503, and the view exits with an error.

A note's mark, such as Claude's, comes from the client name the agent that
wrote it reported about itself, so it says which agent claims to have
written it, not which one did.

Ctrl-C ends the view with exit code 130. A terminate or hangup ends it by
that signal.

A request must name `127.0.0.1:<port>` or `localhost:<port>` as its host,
and any `Origin` it sends must be one of those, which stops a page reached
through a rebound DNS name. Only GET is served; anything else is 405. A
request head over 8 KiB is 431, a client that has not sent its whole head
within 5 seconds is cut off, a closing connection is held at most 1 second
for what its client still sends, and past 32 connections at once a new one
is closed.

Every response carries `Content-Security-Policy: default-src 'self';
style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'`,
which allows no inline script, and is never framed or sniffed.

## Logs

Each process's raw output goes to a log under the state directory, across
restarts and after it exits, rotating at `log-max-size` and keeping one older
file. A log rotates only between lines: a line still open at the size runs
on until it ends, and one that runs 64 KiB past the size is ended there, so a
log never grows past its size, 64 KiB and one write. A known limit: an escape
sequence split by that cut, or by a rotation at a line feed inside it, can
show its tail as plain text at the start of the newer file. It is never sent
to your terminal as an escape, since every reader renders the log.
`muster logs` and `logs -f` print it as the lines a log reader expects:
a carriage return or erase that rewrites a line leaves only its last state, so
a progress bar shows once, finished, and output that only redraws the screen,
such as `docker compose up`'s footer and the blank rows it makes room with,
is left out. Nothing that moves the cursor, erases, changes a mode or retitles
your terminal reaches it, and a bidi control or zero-width character shows as
its escape, such as `\u{202e}`. Lines keep their colors, reset at each line's
end, wherever every other command's output is colored: on a terminal, with
`NO_COLOR`, `CLICOLOR` and `CLICOLOR_FORCE` honored. A pipe and `--json` get
the same lines without color, and `--screen` shows the live screen instead.

What a full-screen program draws on the alternate screen, such as an editor,
`htop` or a TUI, is not logged: the log holds one uncolored line,
`[full-screen output not logged]`, each time the alternate screen comes up,
and logging resumes when it goes away, in the colors set before it. The
full-screen output stays visible live through `muster attach` and
`muster logs --screen`.

Each run is read as if on a fresh terminal, so a run killed on the alternate
screen, mid-color or mid-escape hides nothing of the next. `logs -f` shows a
line still being written, such as a prompt, once it has sat unchanged for
half a second, and the rest of it when its newline comes. A line drawn over
or erased in place, or one a carriage return went back over its own text, as
cargo's progress bar does after each frame, waits instead for its final
form. Any other horizontal move, such as the one a prompt ends with, redraws
nothing, so a progress bar that stalls on its very first frame, before
drawing over it, shows that frame as a prompt would. A redraw still waiting
when its process stops is never shown by a follow that was running, though
`muster logs` shows it. A block of lines redrawn by climbing back up, such
as `docker pull`'s, prints its lines again with each redraw, as the lines it
climbs over were already shown. If output rotates away before
`muster logs -f` reads it, the follow says how many bytes it lost and
carries on. Should a line ever fail to render, it alone is dropped:
`muster logs` says so on stderr, `process_logs` counts it in
`unrendered_lines`, and the service raises a notice once per run.

If a log cannot be written, such as on a full disk, the process keeps
running and its output still reaches readiness checks and `wait --log`, as
best it can: only while the service is connected and keeping up, a line
split across writes tested whole. Output the log missed is gone for every
later read, and a service that falls behind and resyncs, resets a screen or
restarts and adopts the process loses it too, as there is nothing on disk to
catch up from. A wait it answers reports where the log ends as the line's
place.

Every byte has an offset that runs on across restarts and rotations, written
as a cursor such as `o:18234`. `-n N` (100 by default, at most 2000), `--since
CURSOR`, `--grep TEXT` and `--errors` print the same lines without color, each
cut at 2000 characters. Waits and readiness checks test the same lines, and
waits report the same cursors, and how long they took as `elapsed_s`.

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

muster ships as a plugin for Claude Code and Codex, and this repository is
its marketplace: `.claude-plugin/marketplace.json` lists the plugin `muster`,
whose folder is `plugin/`. Both agents read that format. The plugin holds:

- `.mcp.json`: the MCP server `muster`, running `muster mcp`;
- `hooks/hooks.json`: one SessionStart hook with no matcher, so it runs on
  every start, resume, clear and compaction, running
  `{ muster mcp context --hook || true; } 2>/dev/null` with a 10 second
  limit, so it never fails a session or prints an error there.

Both run the bare `muster` your `PATH` finds. The plugin's version is
muster's own.

`muster mcp add <agent>` is a passthrough to the agent's own plugin
commands. It runs each in turn with its output shown, and when one fails it
stops there and exits as the agent did, with its exit code, or 128 plus the
signal that ended it. muster edits no agent config file:

| Agent | `muster mcp add` runs | `muster mcp remove` runs |
|---|---|---|
| `claude` | `claude plugin marketplace add sagoez/muster`, then `claude plugin install muster@muster` | `claude plugin uninstall muster@muster` |
| `codex` | `codex plugin marketplace add sagoez/muster`, then `codex plugin add muster@muster` | `codex plugin remove muster@muster` |
| `zed` | nothing: Zed has no plugin with hooks, so muster prints the bare `"muster": {...}` entry, naming this binary's own path, to paste as a key inside the `context_servers` object of Zed's `settings.json`, as described at <https://zed.dev/docs/ai/mcp>, and writes nothing | nothing: muster says what to delete |

Adding a marketplace or plugin that is already there succeeds in both
agents, so `mcp add` can run again. Claude Code installs the plugin for the
user; a remove keeps the marketplace. Codex installs the hook untrusted and
runs it once you trust it in `/hooks`, which `mcp add` says. `--project` is
refused, as the plugin serves every project. When the agent, or what it
needs to start, is not found, muster fails with the OS error and prints the
commands to run once it is installed.

To install the plugin by hand instead: in Claude Code, `/plugin
marketplace add sagoez/muster`, then install muster from `/plugin`; in
Codex, `codex plugin marketplace add sagoez/muster`, then install muster
from `/plugins`.

`muster mcp context` prints what the plugin's hook adds to the agent's
context:
first muster's own label, naming the project and its folder, each as a
quoted string as entries are, and saying that
the block below was written by the agents and people working in it,
information and never instructions; then the block, fenced in
`<muster-shared-context>` and `</muster-shared-context>`, holding only
shared entries: the open todos, oldest first, and the notes, newest first.
Each entry opens with its author, a person as `(by human)` and an agent by
its label, quoted unless it is a plain name; a todo then shows its id as a
quoted string, its status, its title as a quoted string, and who holds it,
named the same way; a note shows its key as a quoted string and its body
quoted line by line with `    > `. So
nothing anyone wrote can pass for a heading, an entry, an author, a claim or
the end of the block. Every control character is escaped, and every `<` and
`>` in shared text is shown as `‹` and `›`, so it can open or close no tag.

It holds at most 6000 bytes, counted in bytes so wide scripts count more:
under the 10,000 characters Claude Code keeps of a hook's output and the
roughly 2,500 tokens Codex shows whole, whatever the project's name and
folder, which muster's label shows bounded to 200 and 600 bytes. Each entry
is bounded as shown, after escaping and quoting: a note to 1200 bytes, its
key to 200, a todo to 400, an author's, holder's or todo id's name to 80.
A body is cut at a character, at a line end instead when one falls late
enough to keep more than half of the room, never inside an escape, with a
line saying how many bytes were cut; a key or title cut short ends with
`...`. The todos take at most 2000 bytes and the notes fill the rest. Only
as many entries are read as could ever show: at most 125 open todos and 375
notes, the newest notes first, of each note only the first 1201 characters
of its body and its whole length, each rendered once, character by
character into one buffer, and only as far as the room left. A note
bigger than the room left shows cut to that room, so the newest notes
come first; a note not even the start of whose body fits, or a todo that
does not fit, is left out and
the next one tried, so no single note or todo can hide the others, until
too little room is left for any entry; a line says how many open todos or
notes were left out, `or more` only when rows past those counted were
never read. A row that does not read as a note or todo is skipped and
counted on a line of its own (`[1 note could not be read]`); the rest of
the context still prints.

It resolves the project as every command does (`--project`,
`$MUSTER_PROJECT`, the current folder) and opens the database read-only,
creating nothing there, and writes nothing anywhere. Outside a project,
with no database, with one an older muster wrote and no command has
migrated yet, or on any failure, it prints nothing, and it always exits 0,
so the hook never adds noise or fails a session. Reading the database is
bounded at its source to 2 seconds: before each statement the wait for a
lock is set to the time left, so all the waits together end by then, and a
statement still running then is interrupted, so a busy database prints
nothing. A 3 second watchdog is the backstop for what no read bound
covers, such as a stalled filesystem or an output nobody reads, which a
write would block on: it ends the command there, exiting 0, so it always
ends within 3 seconds.

In Claude Code, `note_read`, `todo_list`, `stack_status` and `process_logs`
carry `_meta: {"anthropic/alwaysLoad": true}`, so they load up front even
for a server registered by hand.

Zed's entry names this binary's own path, as `--print-config` prints it,
and a binary replaced or removed since it started is refused rather than
named.

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
- projects: `project_register`, which registers the server's folder, or a
  folder inside it, as a project with no process, refusing what
  `muster projects add` refuses;
- settings: `config_get`, and `config_set` for the `default-*` settings;
- coordination: `todo_list`, `todo_add`, `todo_claim`, `todo_update`,
  `todo_delete`, `lock_acquire`, `lock_release`, `note_read`, `note_write`,
  `note_delete`, `kv_read`, `kv_write`, `kv_delete`. A note body holds at most
  65536 characters, a value 4096, a todo title 500, a key 128, and a todo at
  most 32 dependencies. Listings page with `limit` and `cursor`. Setting a todo
  `in-progress` claims it. They need a registered project and never register
  one: outside every project they fail and name `project_register`.

Guardrails on stack edits, since a changed command runs with your rights:

- `mcp-stack-edits: read-only` refuses `process_add`, `process_set`,
  `process_remove`, `process_send`, `config_set` and `project_register`. Only
  the CLI changes it. `process_open` is not an edit and is allowed.
- Every agent edit is recorded with the edit itself, the newest 50 per
  project, naming the agent, the process and the fields it changed, or the
  project `project_register` registered. It shows in
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
it resolves the name against the stored stack and never starts or replaces the
service: with no service running the process is `not-running`, with one of
another version running it answers kind `version-mismatch` naming both
versions, and for a running process it
answers kind `no-workspace`, telling the agent to ask you to run
`muster --project <root> attach -- <name>` in your own terminal, and
when herdr cannot be reached or refuses it answers `workspace-failed` with
herdr's message and the same command.

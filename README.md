<p align="center"><img src="https://github.com/sagoez/muster/raw/main/docs/logo.svg" width="96" height="96" alt="muster"></p>

# muster

muster runs your project's dev processes (servers, workers, watchers) in the
background, so they keep running whichever terminal you close. You and your
coding agents drive the same stack: you from the CLI, agents over MCP. muster
draws no panes. `muster attach` shows a process in the terminal you are already
in, and inside [herdr](https://herdr.dev) `muster open` puts it in a pane next
to yours.

Beta. Linux and macOS.

https://github.com/user-attachments/assets/adf3cf11-7706-41c2-a00a-f93610f06ad4

Music by @wavetoysmusic - https://wavetoysmusic.com

## Install

You need a recent Rust toolchain.

```sh
cargo install muster-workspace
```

There is no daemon to set up. The first command that needs the background
service starts it.

## Quickstart

```sh
cd ~/code/shop
muster add web --autostart --port 5173 -- npm run dev   # registers the folder as a project
muster add worker --autostart -- bin/worker
muster up              # start the autostart processes
muster ps              # each process's state, ports, uptime and usage, and why one failed
muster logs web -f     # follow the log, across restarts
muster attach web      # see its screen and type into it; ctrl-\ detaches
```

`muster ps` marks each process by how it stands and ends with what to run next:

```
shop  ~/code/shop

■ worker  failed   exit 1 · 2m ago
  Error: connect ECONNREFUSED 127.0.0.1:5432
◇ web     running  :5173 · up 2h 4m · 1.2% · 128.0 MiB

1 failed · 1 running
Run muster logs worker to see why it failed.
```

`muster init` can read `package.json`, `Procfile`, `justfile`, `Makefile` and
compose files and offer the commands it finds. Bare `muster` is `muster ps`.

## What you get

- Processes that outlive the service. Each one runs under its own small holder
  process, so stopping, crashing or upgrading muster leaves them running, and
  the next service picks them back up with their screens intact.
- Restart policies (`never`, `on-failure`, `always`) with backoff, a stop
  signal and grace period per process, and pause/resume.
- Readiness checks: a log line, an open port or an HTTP answer on localhost.
  `muster start --wait` and `muster wait` block until a process is ready, prints
  a line, or reaches a state.
- Logs kept across restarts and after exit, with rotation, `--grep`, `--errors`
  (error lines with their stack traces) and cursors, so you can read only what
  is new since the last look. `muster logs --screen` prints what the terminal
  shows right now.
- Declared ports. A start is refused when something already holds one of the
  process's ports, and muster tells you who holds it and how to free it.
- A shell hook. With `eval "$(muster shell-init bash)"` (or zsh, or fish) in
  your rc file, entering a project's folder starts its autostart processes, and
  once no shell has been in the project for `idle-stop` (30 minutes by
  default) it stops them. The prompt never waits on muster.
- `muster export` writes the stack to a `muster.yml` you can commit, and
  `muster import` loads it into another checkout. Environment values stay out
  unless you ask for them. See [examples/muster.yml](https://github.com/sagoez/muster/blob/main/examples/muster.yml).
- `muster view` serves a live, read-only page of every project's processes on
  127.0.0.1 and opens it in your browser. Only the address it prints opens
  the page, and looking never starts the service.
- `--json` on almost every command, and stable exit codes, for scripts.
- `muster doctor` checks the database, the projects, the shell hook,
  completions and the terminal workspace.

## For agents (MCP)

```sh
muster mcp add claude   # or codex; installs muster's plugin through the agent
muster mcp add zed      # prints the entry for Zed's settings
```

muster ships as a plugin for Claude Code and Codex, and this repository is
its marketplace. The plugin brings muster's MCP server and a SessionStart
hook running `muster mcp context`, which prints the project's notes and
open todos, labelled as information, never instructions, each credited to
the agent that wrote it or to muster's CLI. So every session starts, and
every resumed, cleared or compacted one starts again, knowing what other
agents left. The hook never fails a session. The plugin runs `muster` from
your `PATH`; an agent launched from a GUI needs it on the `PATH` it starts
with.

`muster mcp add` only runs the agent's own commands, with their output
shown, and fails with the agent's exit code; muster edits no agent config
file. It pins the marketplace to this repository's `release` branch,
which every release fast-forwards and a repository ruleset protects, so
the plugin tracks the latest release rather than the binary's exact
version. For Claude Code it runs `claude plugin marketplace add
sagoez/muster#release`, `claude plugin marketplace update muster`, `claude
plugin install muster@muster` and `claude plugin update muster@muster`;
for Codex, `codex plugin marketplace add sagoez/muster@release`, `codex
plugin marketplace upgrade muster` and `codex plugin add muster@muster`.
Running it again moves an installed plugin to the latest release, and
nothing is ever removed. When a step fails, muster names it after the
agent's own error. Codex
runs the hook once you trust it in `/hooks`. `muster mcp remove <agent>`
runs the agent's plugin uninstall. If you registered muster with `claude
mcp add` before, remove that entry with `claude mcp remove muster` (or
`codex mcp remove muster`), or both servers run.

To install it yourself instead, in Claude Code run `/plugin marketplace add
sagoez/muster#release`, then pick muster in `/plugin`; in Codex add the
marketplace as above and pick muster in `/plugins`. Zed has no plugin with hooks, so it
gets the server alone. For any other agent, `muster mcp --print-config`
prints the snippet to paste in. However it is set up, muster's server marks
`note_read`, `todo_list`, `stack_status` and `process_logs` to load up front
in Claude Code instead of behind tool search.

One entry serves every project. Over MCP an agent can:

- see the stack: what runs, readiness, ports, URLs, how each process last
  exited, and a stream of lifecycle events it can long-poll;
- start, stop and restart processes, and wait until they are ready, print a
  line or answer on a port;
- read logs (only errors, or only what is new since a cursor) and the current
  screen, and send input;
- add, change and remove processes, within guardrails: you can make MCP
  read-only with `muster config set mcp-stack-edits read-only`, every agent
  edit shows up in `muster notices`, and a working directory has to stay
  inside the project;
- coordinate with other agents through shared notes, todos, key-value state
  and locks on the project. A repo with nothing to run needs no process for
  that: `muster projects add` (or the agent's `project_register`) registers
  the folder as a project on its own;
- open a process in a new herdr pane beside its own, without taking focus.

Crashes and readiness timeouts ride along on every result as `alerts`, so an
agent hears about a crash without polling. Stopping the whole stack is left to
you at the CLI.

## herdr

From a herdr pane, `muster open web` splits a new pane, focuses it and attaches
to `web`. Detaching closes the pane. Without `--direction right|down`, it
splits right when the pane is wider than tall and each half keeps at least 40
columns, and down otherwise. If herdr cannot be reached, `open`
fails with herdr's message and points you at `muster attach`; over MCP,
`process_open` answers `workspace-failed`. Outside herdr, `muster open` is
`muster attach`.

## How it works

```mermaid
flowchart LR
    you([you]) --> cli[muster CLI]
    agent([agents]) --> mcp[muster mcp]
    hook[shell hook] -- "which shell is where" --> svc
    cli --> svc[per-user service]
    mcp --> svc
    cli --> db[(muster.db)]
    mcp --> db
    svc --> db
    subgraph held["one holder per process: owns its PTY, screen and log, outlives the service"]
        h1[holder] --> p1["web: npm run dev"]
        h2[holder] --> p2["worker: cargo run --bin worker"]
    end
    svc --> h1
    svc --> h2
    cli -- "attach" --> h1
    cli -- "open" --> herdr[herdr pane]
    mcp -- "process_open" --> herdr
```

`web` and `worker` stand for your own processes, whatever `muster add` gave
the project.

There is one background service per user. It owns a supervisor for each
loaded project, applies restart policies, runs readiness checks and keeps the
event stream. The processes themselves run under holders: one detached muster
process per child, which owns that child's PTY, terminal
emulator and log, and keeps serving until a supervisor has collected the
child's exit. The service talks to
holders over Unix sockets, so it can stop or crash or be upgraded, and the next
one reconnects to every holder and carries on.

Everything muster stores sits in one SQLite file, `muster.db` in the state
directory: each project's stack, the machine's settings, the agents' notes,
todos and locks, and how each process last ended. The CLI and the MCP server
write stack edits there directly and then tell the service what changed, so
editing works even when the service is down. `attach` connects your terminal
straight to the holder's screen, and `open` asks herdr for a pane that runs
`attach`.

Inside, the code is hexagonal, and the hexagon is a Cargo workspace of
`muster-*` crates. A pure `muster-domain` with no I/O defines the ports,
`muster-application` holds the use cases, and everything concrete (CLI, MCP,
service, holder, PTY, SQLite) is an adapter in its own crate, wired together in
`muster-workspace`'s `main.rs`.

## Reference

Every command, setting, exit code, the JSON conventions, the stack file format
and the full MCP tool list are in [docs/reference.md](https://github.com/sagoez/muster/blob/main/docs/reference.md).

## License

MIT

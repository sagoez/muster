# muster

A terminal workspace for running CLI agents and dev processes side by side.

muster runs your agents, dev servers, log tails, and build watchers as panes in a
single terminal and manages their lifecycle: start, stop, restart, and
auto-restart on failure.

It's been my daily driver for running local work for a while. This is the
cleaned-up cut I opened up, done with help of the AI.

## Features

- Runs each process under its own PTY and renders it live. A process that exits
  keeps its last screen.
- Full lifecycle control: start/stop, restart, pause/resume (SIGSTOP/SIGCONT), and
  auto-restart on failure.
- A projects tree in the sidebar for switching between workspaces.
- Live config reload: edits are reconciled into the running workspace, adding new
  processes and dropping removed ones while leaving running ones untouched.
- Every option in the config is also settable from the TUI.
- A failed process raises an alert visible from any pane.

## Getting started

Requires a recent Rust toolchain.

```sh
cargo run                     # starts the TUI on ./muster.yml
cargo run -- --config my.yml  # use a different config
```

Press `?` in the app for the full keymap:

- `j`/`k` or arrows to move, `Enter`/`l` to open, `h` to go back
- `s` start/stop, `r` restart, `p` pause, `x` stop, `t` toggle autostart
- `a` add a process, `n` new project, `o` switch projects, `d` remove a project
- `C-a` detaches from a focused pane; the same commands work as `C-a` chords while
  attached
- `q` to quit

## Configuration

A workspace is a YAML file with three sections: `agents`, `terminals`, and
`commands`. They behave identically; the grouping only controls how they appear in
the sidebar.

```yaml
agents:
  - name: claude
    command: claude
    description: coding agent
    autostart: false

terminals:
  - name: shell
    command: null          # null runs your login shell
    description: your login shell

commands:
  - name: clock
    command: while true; do date +%T; sleep 1; done
    restart: on_failure
    autostart: false
```

- `autostart`: `null` uses the default (agents and terminals start with the
  workspace, commands wait for `s`), or set `true`/`false` explicitly. Toggle it
  live with `t`.
- `restart`: `on_failure`, `always`, or `null` to never restart.
- `working_dir`: launch directory; inherits the workspace directory when `null`.

## muster run

`muster run` registers a command into a project and runs it in place, without
opening the config:

```sh
muster run -- npm run dev
muster run --name api --kind terminal -- cargo watch -x run
```

The target is `--project` if given, otherwise `$MUSTER_PROJECT` (exported into
every pane), otherwise `--config`. Shell quoting is preserved and `--project` has
tab completion.

## Status

Single user, Unix only.

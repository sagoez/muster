# AGENTS.md - muster

> Single source of truth for how we build this repo. `CLAUDE.md` is a symlink to
> this file. These rules are self-contained: do not defer to, inherit from, or
> sync with any AGENTS.md / CLAUDE.md from other repositories.

## Project Overview

**muster** - a terminal UI, built with [ratatui](https://ratatui.rs) + crossterm,
for an agentic terminal workspace. It runs multiple CLI agents and dev processes
side by side, manages a process/stack (start/stop/restart, auto-restart,
file-watch triggers, port waits), detects agent work/idle state, and exposes
shared coordination primitives (todos, scratchpads, locks, timers, key-value).

> Architecture and the config schema are agreed in discussion before
> implementation. This file governs *how* we write the code, not *what* it does.

## Architecture: Ports and Adapters (hexagonal)

The code is organized as a hexagon. The domain is pure and knows nothing about
ratatui, crossterm, tokio, PTYs, or YAML. Everything concrete is an adapter that
plugs into a port.

- **`domain`** - the core. Entities, value objects, and **ports** (traits the
  core needs the outside world to fulfil, e.g. `ProcessRunner`, `ConfigSource`,
  `Clock`). No I/O, no framework types, no `async`. Fully unit-testable in
  isolation.
- **`application`** - use-cases that orchestrate the domain through ports (e.g.
  `WorkspaceService`). Depends on `domain` only.
- **`adapter`** - infrastructure. Split into:
  - **driving adapter** `adapter/tui` - crossterm input + ratatui render + the
    runtime loop; it drives the application.
  - **driven adapters** `adapter/pty` (portable-pty implements `ProcessRunner`),
    `adapter/config` (serde_yaml implements `ConfigSource`), etc.
- **`main.rs`** - the composition root. The only place that knows every concrete
  adapter; it constructs them, wires them to ports, and runs.

**Dependency rule (strict):** dependencies point inward. `domain` depends on
nothing internal. `application` depends on `domain`. `adapter` depends on
`domain` (and its own crates). `main` depends on everything and wires it. A port
is a trait defined in `domain`; an adapter is an `impl` of that trait living in
`adapter`. Framework/library types never cross into `domain`.

```
src/
  main.rs              # composition root
  error.rs             # MusterError (transparent aggregate of domain/adapter errors)
  constants.rs         # crate-wide named constants (app identity, ...)
  domain/
    process/           # Process, ProcessKind, ProcessState (entities + value objects)
    port/              # ProcessRunner, ConfigSource, Clock, ... (traits)
  application/
    workspace.rs       # use-case services
  adapter/
    tui/               # driving: input + render + runtime loop
    pty/               # driven: ProcessRunner via portable-pty
    config/            # driven: ConfigSource via serde_yaml
```

## Dependencies and libraries

- **Do not reinvent the wheel.** If a crate does the job, use it instead of
  hand-rolling (PTY handling, VT parsing, newtype validation, backoff, arg
  parsing, file-watching, ...).
- **Prefer maintained crates.** A crate that is quiet but feature-complete and
  stable is fine; a crate that is genuinely stale relative to the ecosystem is
  not. Use judgement, and prefer the actively-maintained option when choosing.
- **Roll your own only when it genuinely fits** - the logic is small and
  domain-specific, or no maintained crate covers it well. A thin, tested module
  beats a heavy or abandoned dependency.
- Keep framework crates out of `domain`; they belong to adapters.

Chosen libraries (extend deliberately, same rules apply):

| Concern | Crate |
|---|---|
| TUI render | `ratatui` |
| Terminal I/O | `crossterm` |
| Event loop | std threads + `std::sync::mpsc` |
| PTY | `portable-pty` |
| VT parse + embed | `vt100` + `tui-term` |
| Validated newtypes | `nutype` |
| Restart backoff | `backon` |
| Arg parsing | `clap` |
| File watching | `notify` |
| Config (de)serialize | `serde` + `serde_yaml_ng` |
| Errors | `thiserror` |
| Builders / getters | `typed-builder` + `getset` |
| Enum ergonomics | `strum` |
| Snapshot tests | `insta` |

## No magic values

- **Never write a bare meaningful literal.** Every meaningful string or number
  becomes a named `const`, an enum variant, or a config value. This includes
  durations, sizes, limits, keybindings, glyphs, labels, titles, colors, section
  names, and file paths.
- **Co-locate constants near use.** Module-specific constants live in that
  module; only genuinely crate-wide constants (app identity, etc.) live in
  `constants.rs`. Do not build one giant global dumping ground.
- Trivially-obvious, non-domain literals (`0`, `1`, an index step, empty string)
  are fine; the rule targets values that carry meaning.

## Code Style

### Comments and Documentation
- **No trivial comments.** No obvious or redundant comments (e.g. `// set x`,
  `// return the result`). Comment the non-obvious *why*, never the *what*.
- **Doc-comment every function.** Every `pub` or internal function gets a `///`
  describing purpose, parameters, and return. Add `# Errors` for fallible fns.
- **Keep doc comments succinct** - a line or two by default; add length only when
  the complexity earns it.
- **Never use em dashes (-) in any file.** Use a plain hyphen. Applies to code,
  comments, docs, and commit messages.

### No dead code (hard rule)
- **Never leave stranded dead code.** Remove unused functions, methods, consts,
  fields, enum variants, imports, and modules the moment they stop being used.
  Never comment code out or keep it "just in case" - deleting is free and git
  remembers.
- Code orphaned by a refactor goes in the same change: if you rename or replace
  something, delete the old version immediately, not later.
- `pub` items in the library crate do not trip `dead_code`, so when you remove a
  consumer, check whether what it used is now unused and remove that too.

### Imports
- **Use normal `use` imports, never inline qualified paths** like
  `std::io::Error`.
- Formatting owns import order. `just fmt` runs nightly rustfmt with
  `group_imports = "StdExternalCrate"` and `imports_granularity = "Crate"`:
  imports are grouped std -> external -> crate, merged per crate root, and sorted
  within each group. Do not reorder by hand; run the formatter.

### Enums over dangling strings
Prefer enums over raw string literals for any fixed set of domain values compared
or matched (process state, process kind, pane focus, key actions, ...). Give them
`Display`/`FromStr` via `strum` or a manual impl. Never stringly-typed.

### Newtypes over primitives (via `nutype`)
Wrap meaningful primitives in validated newtypes rather than passing bare
`String`/`u64`. Use `nutype` for sanitization + validation instead of
hand-rolling; only hand-write a newtype when `nutype` cannot express the rule.

```rust
use nutype::nutype;

#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(Debug, Clone, PartialEq, Eq, AsRef, Display, Serialize, Deserialize),
)]
pub struct ProcessName(String);
```

Add TUI value objects (`PaneId`, `Cols`, `Rows`) the same way rather than passing
bare `u16`/`usize`.

### Structs
- **Always derive `TypedBuilder` + `Getters`** on data structs, at minimum,
  including render state, widget state, and event structs. The exceptions are
  RAII resource guards and stateful runtime controllers whose construction wires
  side-effecting resources (e.g. the terminal guard, the event-loop `App`): these
  use a hand-written constructor, and a small private helper struct local to such
  a controller may use a plain struct literal.
- **Config structs additionally derive `WithSetters`, `Serialize`,
  `Deserialize`** with `#[set_with]` and `#[getset(get = "pub", set_with =
  "pub")]`.
- Fields are private; expose access through `Getters`, never `pub` fields.

```rust
#[derive(Clone, Serialize, Deserialize, Debug, Getters, WithSetters, TypedBuilder)]
#[set_with]
pub struct ProcessConfig {
    #[getset(get = "pub", set_with = "pub")]
    name: ProcessName,
    #[getset(get = "pub", set_with = "pub")]
    command: CommandLine,
}
```

### Config defaults
- **Never use `#[serde(default)]` on config fields.** All defaults live in the
  shipped config file, not in Rust, so every value is explicit and visible.

### Secret config fields
Secrets must use a non-empty newtype, add `#[serde(skip_serializing)]`, and use a
custom `Debug` that prints `[REDACTED]` (never `#[derive(Debug)]`).

### Error handling
- Use `thiserror` for all error types.
- Each domain/adapter module defines its own error enum with descriptive
  `#[error("...")]` messages.
- The top-level `MusterError` in `crate::error` is a lean aggregate: just
  `#[error(transparent)]` + `#[from]`. Let each error's own `Display` speak.
- Fallible operations return `Result`, never panic via `.expect()`/`.unwrap()`.
  The only tolerated panics are in tests and a top-level panic hook that restores
  the terminal before reporting.

### Prefer methods, but free functions are fine when they fit
Default to methods on a struct. But leave a free function free when it reads
better: pure helpers with no natural owner, small stateless utilities,
conversion/parse helpers. Do not invent a struct just to host one function.

### Module structure
A folder is for **grouping**, not decoration.
- **Single-file module -> flat file** (`foo.rs`, never `foo/mod.rs` alone). A
  folder containing nothing but `mod.rs` is forbidden.
- **Multiple sibling files -> folder**, where `mod.rs` only declares and
  re-exports. One type per file.
- **Avoid module inception.** Never name a child module the same as its folder
  (`clippy::module_inception`); name the aggregate's file for its role, e.g.
  `entity`, not the folder.

```rust
// domain/process/mod.rs
mod entity;
mod kind;
mod state;

pub use entity::*;
pub use kind::*;
pub use state::*;
```

## TUI Conventions

### Rendering
- Rendering is pure: `render(frame, area, state)` reads immutable state and
  draws. Never mutate domain state inside a render.
- Prefer `StatefulWidget` for scroll/selection; keep widget state minimal, derive
  the rest from domain state.
- Redraw on change, not a busy spin.

### PTY handling
- Spawn children under a PTY via `portable-pty`. Parse output through `vt100`
  into a screen/scrollback model; ratatui (via `tui-term`) renders that model.
- PTY read/write runs off the render path; communicate over channels, never
  shared locks on the render path. The runtime loop owns the parsers; reader
  tasks only shuttle bytes and lifecycle events as messages.

### Event loop
- A single synchronous loop owns the state. Terminal input, PTY output, and ticks
  arrive as messages on one `std::sync::mpsc` channel: a dedicated thread blocks
  on crossterm input, each process's reader thread forwards output, and the loop
  drains the channel and redraws. No async runtime.
- Terminal raw-mode/alt-screen setup is a RAII guard that restores on drop and on
  panic. Never leave the terminal in raw mode on exit.

### Testing
- **Tests live inline**, in `#[cfg(test)] mod tests { ... }` in the same file. Do
  not use the `tests/` directory for unit tests.
- **Snapshot ratatui output with `insta`**: render into a `TestBackend` buffer
  and snapshot it. Review with `cargo insta review`; never blindly accept.
- **Use getters** to read private fields in tests rather than making fields `pub`.
- Pure domain logic (state machines, config parsing) has direct unit tests with
  no terminal involved.

## Commands

Use the `justfile`:

```bash
just build        # cargo build
just check        # cargo check
just test         # cargo test
just run          # run the TUI (just run -- --config muster.yml)
just fmt          # format (nightly rustfmt, honors rustfmt.toml)
just fmt-check    # verify formatting
just lint         # cargo clippy --all-targets -- -D warnings
just ci           # fmt-check + lint + test
```

Formatting requires nightly rustfmt (for the unstable import options). `just fmt`
locates the installed nightly toolchain and overrides `RUSTFMT`; it does not
change any global tool config.

## Git

- **Never add AI attribution to commits or PRs** - no `Co-Authored-By`, no
  "Generated with" footer. Commit text reads as if the author wrote it.
- **Never use `git checkout` / `git restore`** (or anything that discards
  uncommitted changes) without asking first.
- **Never commit or push on your own.** New work stays uncommitted until the
  user explicitly asks for a commit - no exceptions, not even when a task is
  finished and verified. Completing work and committing it are separate steps;
  only the user starts the second.

## Environment

- **Do not install software globally or change global tool config** without
  explicit permission (no `mise use -g`, `rustup default`, `apt`, `npm -g`, ...).
- When a specific tool/version is needed, use an isolated per-project mechanism.

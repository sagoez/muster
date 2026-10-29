# AGENTS.md - muster

> Single source of truth for how we build this repo. `CLAUDE.md` is a symlink to
> this file. These rules are self-contained: do not defer to, inherit from, or
> sync with any AGENTS.md / CLAUDE.md from other repositories.

## Project Overview

**muster** - the supervisor and control plane behind your terminal. A headless
per-user service runs each project's dev processes (servers, workers, watchers)
under their own holder processes, keeps their logs and screens, and restarts
them by policy. One SQLite database, `muster.db` in the state directory, holds
every project's stack, the machine's settings, the shared coordination state
(notes, todos, key-value, locks) and a short log of the stack edits agents
made. A project is a folder, registered by the first
process added to it. Users and agents drive muster through the CLI and an MCP
server; `muster attach` shows a process in the user's own terminal. muster draws
no panes, and agents and interactive terminals live in herdr.

> Architecture and the database schema are agreed in discussion before
> implementation. This file governs *how* we write the code, not *what* it does.

## Compatibility (pre-1.0)

Until muster reaches v1.0.0, breaking changes are allowed. Holder records, the
export file and the service protocol need no backward-compat shim: do not add
serde defaults, version bumps, or fallbacks purely to keep an old file readable.
Ignore code-review findings whose only concern is backward compatibility with
pre-1.0 on-disk files.

The database is the exception: it is the only copy of the user's stack, so it
migrates forward. Its schema version is `PRAGMA user_version`, and each schema
change is a new numbered SQL file under `src/adapter/store/migrations/`. A
shipped migration is never edited.

## Architecture: Ports and Adapters (hexagonal)

The code is organized as a hexagon. The domain is pure and knows nothing about
sockets, tokio, PTYs, terminals or YAML. Everything concrete is an adapter that
plugs into a port.

- **`domain`** - the core. Entities, value objects, and **ports** (traits the
  core needs the outside world to fulfil, e.g. `ProcessRunner`,
  `StackStore`, `Notifier`). `LocalState` is every store port as one handle.
  No I/O, no framework types, no `async`. Fully unit-testable in isolation.
- **`application`** - use-cases that orchestrate the domain through ports
  (`StackEditor`, `StackSync`, `Workspace`, `ProcessLifecycle`). Depends on
  `domain` only.
- **`adapter`** - infrastructure:
  - **driving adapters** `adapter/cli` (clap commands, `attach`) and
    `adapter/mcp` (the rmcp server). Both edit stored state through
    `StackEditor` and drive the service through `adapter/client`, which holds
    the service client, the process use cases, their reports and views, and
    `ClientError`; each driver maps that error to its own surface and neither
    imports the other. Each takes its state and the service as ports
    (`LocalState`, `ServiceAccess`) from the composition root, so a test
    injects fakes and never reaches the user's real database or socket.
  - **`adapter/service`** - the per-user service: one runtime loop owning a
    supervisor per loaded project, the socket protocol and notices. It reads
    the database through read-only ports (`StackReader`, `SettingsStore`,
    `StackEditLog`); writers tell it what changed.
  - **`adapter/holder`** - the detached process that owns one child's PTY,
    emulator and log, and outlives the service.
  - **driven adapters** `adapter/pty` (portable-pty implements
    `ProcessRunner`), `adapter/store` (one `SqliteStore` implements every store
    port), `adapter/exchange` (the export file), `adapter/detect` (stack
    detection for `muster init`), `adapter/terminal` (the VT emulator and
    keyboard tracker), notifier, metrics.
- **`main.rs`** - the composition root. The only place that knows every concrete
  adapter; it constructs them, wires them to ports, and runs. It hands the CLI
  its ports through `CliPorts`, and builds the MCP server's deps and runtime
  itself when the CLI asks to serve one.

**Dependency rule (strict):** dependencies point inward. `domain` depends on
nothing internal. `application` depends on `domain`. `adapter` depends on
`domain` (and its own crates). `main` depends on everything and wires it. A port
is a trait defined in `domain`; an adapter is an `impl` of that trait living in
`adapter`. Framework/library types never cross into `domain`.

```
src/
  main.rs              # composition root: CliPorts, the MCP server, service and holder entry points
  lib.rs               # the library crate the binary and integration tests link
  error.rs             # MusterError (transparent aggregate of domain/adapter errors)
  constants.rs         # crate-wide named constants (app identity, ...)
  domain/
    process/           # Process, ProcessState, restart and stop policies, readiness checks
    config/            # Stack, ProcessSpec, ProcessDraft, ProcessChange, StackEdit, settings keys
    coordination/      # shared notes, todos, key-value entries and locks
    pty/               # SpawnRequest, PtySize, process output and exit outcomes
    port/              # ProcessRunner, StackStore, StackReader, LocalState, Notifier, ... (traits)
    value/             # validated newtypes (ProcessName, ProjectRoot, EnvName, ...)
    project.rs, settings.rs, color_scheme.rs, notification.rs
  application/         # StackEditor, StackSync, Workspace, ProcessLifecycle
  adapter/
    cli/               # driving: commands, attach
    mcp/               # driving: the MCP server over stdio
    client/            # what both drivers share: ServiceAccess, process use cases, reports
    service/           # the per-user service and its supervisors
    holder/            # one detached process per child, owning its PTY
    terminal/          # VT emulator, keyboard tracker, the bounded vte parser
    pty/               # driven: ProcessRunner via portable-pty
    store/             # driven: SqliteStore, migrations
    exchange/          # driven: the `muster export` / `muster import` file
    detect/            # driven: stack detection for `muster init`
    wire/              # the DTOs domain values travel as, and their serde with-modules
    workspace_request.rs  # the MCP-to-service requests the service protocol lacks
    sanitize.rs        # control characters made safe to print, for the service and the CLI
    ipc.rs, private_dir.rs, no_state_dir.rs, ...  # sockets, the state directory, OS probes
```

## Dependencies and libraries

- **Do not reinvent the wheel.** If a crate does the job, use it instead of
  hand-rolling (PTY handling, VT parsing, newtype validation, backoff, arg
  parsing, atomic writes, ...).
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
| Raw mode, size and polling on a given terminal fd; rlimits, sessions, process signals | `rustix` |
| What rustix lacks: `strsignal` text and signal dispositions in tests | `libc` |
| Unix signals | `signal-hook` |
| Event loops | std threads + `crossbeam-channel` |
| PTY | `portable-pty` |
| VT parse + embed | `alacritty_terminal` |
| Validated newtypes | `nutype` |
| Byte sizes | `bytesize` |
| Arg parsing and completions | `clap` + `clap_complete` |
| CLI styling | `anstream` + `anstyle` |
| Interactive CLI frames, spinners and prompts | `cliclack` (themed through `console`), called only by `adapter/cli`'s presenter |
| MCP server and its tool schemas | `rmcp` on `tokio`, `schemars` |
| Desktop notifications | `notify-rust` |
| Process usage | `sysinfo` |
| Listening sockets of a process tree, and who holds a port | `listeners` |
| Readiness and wait HTTP probes | `ureq` |
| Log and wait patterns | `regex` |
| Shell words of a command line | `shlex` |
| Platform state and config directories | `directories` |
| Service version comparison | `semver` |
| Todo and holder ids | `uuid` |
| Display width and text normalization | `unicode-width`, `unicode-normalization`, `unicode-properties` |
| State: projects, stacks, settings, coordination, the edit log | `rusqlite` |
| Export file and wire (de)serialize | `serde` + `serde_yaml_ng`, `serde_json` |
| Durations | `humantime` + `humantime-serde` |
| Atomic file writes | `atomic-write-file` |
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
  `std::io::Error`. A crate-level function called through its crate
  (`serde_json::to_string`) is fine.

### Unsafe and the OS
- Reach the OS through `rustix` wherever it covers the call. A direct `libc`
  call is only for what rustix lacks, and every `unsafe` block carries a
  `// SAFETY:` comment saying why it holds.
- Formatting owns import order. `just fmt` runs nightly rustfmt with
  `group_imports = "StdExternalCrate"` and `imports_granularity = "Crate"`:
  imports are grouped std -> external -> crate, merged per crate root, and sorted
  within each group. Do not reorder by hand; run the formatter.

### Enums over dangling strings
Prefer enums over raw string literals for any fixed set of domain values compared
or matched (process state, restart policy, process action, error kind, ...). Give them
`Display`/`FromStr` via `strum` or a manual impl. Never stringly-typed.

### Enum derivations
Use `strum` derives and iterator traits for enum display, parsing, and variant
enumeration whenever they express the behavior. Do not hand-maintain option
arrays or `match` implementations for those mechanical conversions.

**Enum case is always kebab-case.** Whichever of `strum` or `serde` an enum
uses, its case attribute is kebab-case: `#[strum(serialize_all =
"kebab-case")]`, `#[serde(rename_all = "kebab-case")]`. Never `lowercase`,
`snake_case` or any other casing.

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

Terminal value objects (`PaneId`, `Cols`, `Rows`) follow the same rule rather
than passing bare `u16`/`usize`.

### Structs
- **Always derive `TypedBuilder` + `Getters`** on data structs, at minimum,
  including protocol messages, status views, and event structs. The exceptions
  are RAII resource guards and stateful runtime controllers whose construction
  wires side-effecting resources (e.g. `RawModeGuard`, `ServiceRuntime`,
  `HolderServer`, `Supervisor`): these use a hand-written constructor, and a
  small private helper struct local to such a controller may use a plain struct
  literal.
- **Domain definitions are not serde types.** `ProcessSpec`, `Settings` and
  their kin derive `WithSetters` with `#[set_with]` and `#[getset(get = "pub",
  set_with = "pub")]` where they are edited by value. The export file has its
  own serde DTOs in `adapter/exchange`, each with `#[serde(deny_unknown_fields)]`
  and converted to and from the domain with `TryFrom`/`From`. A domain value on
  a wire or in a file names its DTO through `adapter::wire::WireForm`, and the
  field keeps its domain type with `#[serde(with = "...::wire::form")]`
  (`optional_form`, `form_list`). Only validated newtypes derive serde.
- Fields are private; expose access through `Getters`, never `pub` fields.
  The one exception is a deps struct (`SupervisorDeps`, `ServiceDeps`):
  its boxed ports are moved out once by the controller it builds, which
  destructures it, so its fields are `pub(super)` and it has no getters.

```rust
#[derive(Clone, Debug, PartialEq, Eq, Getters, WithSetters, TypedBuilder)]
#[set_with]
pub struct ProcessSpec {
    #[getset(get = "pub", set_with = "pub")]
    name: ProcessName,
    #[getset(get = "pub", set_with = "pub")]
    command: CommandLine,
}
```

### Defaults
- **Defaults live in the database, never in Rust or serde.** The initial
  migration seeds the settings row, and a new process takes the `default_*`
  settings for every field `muster add` leaves out, so every stored value is
  explicit. Never use `#[serde(default)]`: an optional field is an `Option`,
  and a field that tells absent from `null` is read by hand.
- **Protocol bounds are named constants, not defaults.** How long an MCP
  call may block, a page's default and largest size (`DEFAULT_LIST_LIMIT`,
  `MAX_LIST_LIMIT`) and a log read's size bound what one call costs, not what
  the user configures, so they are constants beside the code that enforces
  them. A tool description names them through `adapter/mcp/described.rs`
  placeholders, never by repeating the number.

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
  The only tolerated panics are in tests and a top-level panic hook that gives
  an attached terminal back before reporting.

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
mod restart;
mod state;

pub use entity::*;
pub use restart::*;
pub use state::*;
```

## Service and holder

### Single-owner event loops
- The service and each holder run one synchronous loop that owns all of their
  state: `ServiceRuntime` owns every project's `Supervisor`, `HolderServer` owns
  its child's emulator, keyboard tracker and log. Every other thread (socket
  acceptor, client readers, PTY reader, timers) only sends
  events to that loop over a `crossbeam-channel`; none of them touches the state.
- Nothing blocking runs on a loop. A socket read, a holder round trip or a
  bounded wait goes to a helper thread whose result comes back as an event.
- The only async runtime is the one `rmcp` needs inside `muster mcp`. It is
  shut down in the background once the client disconnects, so a tool call
  still blocking never keeps the server alive.
- Every vte parser goes through `BoundedParser`, which drops a sequence that
  runs past `MAX_PENDING_BYTES` without dispatching, so no child can grow a
  parser's buffer without bound.

### Processes outlive the service
- Every child runs under its own holder, launched detached from the muster
  binary. A service stop, crash or upgrade never touches a running child: the
  next service adopts the live holders and rebuilds each screen from them.
- On exit the service waits out launches still in flight, leaves every holder
  running, removes its socket and releases its lock before it answers the
  client that stopped it. A holder serves until a supervisor has collected its
  child's exit.

### PTY handling
- Spawn children under a PTY via `portable-pty`. Parse output through
  `alacritty_terminal`, wrapped by `TerminalEmulator`, into a screen and
  scrollback model the holder repaints onto any client that attaches.
- PTY read/write runs off the loop; reader threads only shuttle bytes and
  lifecycle events as messages, never shared locks on the loop's state.
- `muster attach` puts the user's terminal in raw mode through a RAII guard that
  restores on drop and on panic. Never leave the terminal in raw mode on exit.

### Testing
- **Tests live inline**, in `#[cfg(test)] mod tests { ... }` in the same file.
  The `tests/` directory holds only integration tests that run the built binary.
- **Snapshot CLI output with `insta`**: help text, tables and JSON. Review with
  `cargo insta review`; never blindly accept.
- **Use getters** to read private fields in tests rather than making fields `pub`.
- Pure domain logic (state machines, config parsing) has direct unit tests with
  no terminal involved.
- Tests that spawn a service or holder use isolated state directories and stop
  every process they start.

## Commands

Use the `justfile`:

```bash
just build        # cargo build
just check        # cargo check
just test         # cargo test
just run          # run the CLI (just run -- ps, just run -- up)
just fmt          # format (nightly rustfmt, honors rustfmt.toml)
just fmt-check    # verify formatting
just lint         # cargo clippy --all-targets -- -D warnings
just ci           # fmt-check + lint + test
```

Formatting requires nightly rustfmt (for the unstable import options). `just fmt`
locates the installed nightly toolchain and overrides `RUSTFMT`; it does not
change any global tool config.

## Review gate

Every new feature passes this gate before it is called done:

1. **Four adversarial reviewers**, run in parallel as independent subagents.
   Each gets the full diff and one lens: correctness and edge cases;
   concurrency, process lifecycle and failure paths; security and untrusted
   input (MCP, IPC, the database, the export file); architecture and the rules
   in this file.
   Each reviewer tries to break the change, and every finding is verified
   against the code before it is acted on. Each finding that survives owes a
   regression test that fails without its fix.
2. **`/code-review`** on the same diff after the adversarial findings are fixed.
3. Repeat both until a full round surfaces nothing new, then run `just ci`.

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

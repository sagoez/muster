use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand};
use muster::{
    adapter::{
        cli::{self, RunArgs},
        config::{YamlConfigSource, YamlProjectRegistry, YamlSettingsStore},
        notifier::DesktopNotifier,
        path::FsPathCompleter,
        pty::PortablePtyRunner,
        tui::{self, Adapters, TerminalGuard},
    },
    application::Workspace,
    constants::APP_NAME,
    domain::port::ConfigSource,
    error::Result,
};

/// Conventional workspace filename used when no explicit path is supplied.
const DEFAULT_CONFIG_FILE: &str = "muster.yml";

/// Command-line arguments. With no subcommand, muster launches its TUI.
#[derive(Parser)]
#[command(about = "A terminal workspace for running CLI agents and dev processes")]
struct Args {
    /// Path to the workspace config file. Global, so it is recognized before or
    /// after a subcommand rather than being swallowed by `run`'s command args.
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

/// Subcommands. Absent, muster runs the TUI.
#[derive(Subcommand)]
enum Command {
    /// Register a command in a project, then run it.
    Run(RunArgs),
}

/// Entry point: dispatches to the `run` capture command or, by default, the TUI.
///
/// # Errors
/// Returns an error if the config cannot be loaded or the terminal fails.
fn main() -> Result<()> {
    // When invoked by a shell's completion hook this generates candidates and
    // exits; otherwise it returns and normal parsing proceeds.
    clap_complete::CompleteEnv::with_factory(Args::command).complete();
    let args = Args::parse();
    match args.command {
        Some(Command::Run(run_args)) => run_capture(
            run_args,
            args.config
                .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_FILE)),
        ),
        None => run_tui(args.config),
    }
}

/// Adds a command to a project and runs it in place, reporting a friendly error
/// and exiting non-zero on failure. `config` is the top-level `--config`, used
/// as the target when neither `--project` nor `$MUSTER_PROJECT` is given.
fn run_capture(args: RunArgs, config: PathBuf) -> Result<()> {
    let registry = YamlProjectRegistry;
    if let Err(error) = cli::run(args, config, &registry) {
        eprintln!("{APP_NAME}: {error}");
        std::process::exit(1);
    }
    Ok(())
}

/// Composition root for the TUI: resolves and loads the workspace config, wires
/// its adapters, and runs it under a restoring terminal guard.
///
/// # Errors
/// Returns an error if the config cannot be loaded or the terminal fails.
fn run_tui(explicit_config: Option<PathBuf>) -> Result<()> {
    install_panic_hook();
    let registry = YamlProjectRegistry;
    let current_project = cli::current_project_from_env();
    let local_config = PathBuf::from(DEFAULT_CONFIG_FILE);
    let config_path = cli::resolve_tui_config(
        explicit_config.as_deref(),
        current_project.as_deref(),
        &local_config,
        &registry,
    )?;
    let config = YamlConfigSource::builder()
        .path(config_path.clone())
        .build();
    let workspace = Workspace::builder()
        .processes(config.load()?.to_processes())
        .build();
    let adapters = Adapters::builder()
        .runner(Box::new(PortablePtyRunner))
        .registry(Box::new(registry))
        .completer(Box::new(FsPathCompleter))
        .notifier(Box::new(DesktopNotifier::new()))
        .settings_store(Box::new(YamlSettingsStore))
        .build();
    let mut guard = TerminalGuard::new()?;
    tui::run(&mut guard, workspace, adapters, config_path)
}

/// Installs a panic hook that restores the terminal before delegating to the
/// previous hook, so a panic never leaves the user stuck in raw mode.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = TerminalGuard::restore();
        previous(info);
    }));
}

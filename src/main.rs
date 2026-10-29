use std::{env, panic, path::Path, process, sync::Arc};

use muster::{
    adapter::{
        cli::{self, CliPorts},
        holder::{ExecHolderLauncher, HolderDirs, HolderRunner, run_holder},
        metrics::SysinfoMetrics,
        notifier::DesktopNotifier,
        private_dir::{prepare_state_dir, state_dir},
        service::{
            ProjectPorts, ServiceDeps, ServiceError, ServiceRuntime, Singleton, protocol,
            redirect_stderr,
        },
        store::SqliteStore,
    },
    constants::APP_VERSION,
    domain::{
        config::ConfigError,
        port::{Notifier, ProcessMetrics},
    },
    error::MusterError,
};

/// What a service started beside a running one records before it exits, so
/// each start that lost the race is visible in the service log.
const SERVICE_ALREADY_RUNNING: &str =
    "another muster service already runs for this state directory; this one exits";

/// Entry point: parses the command line and runs it on the concrete
/// adapters wired here.
fn main() {
    restore_terminals_on_panic();
    let args = cli::parse_args();
    cli::run(args, &LocalPorts);
}

/// Installs the top-level panic hook: a terminal `muster attach` holds in
/// raw mode is given back before the panic is printed, so the message is
/// readable and the shell usable. A panic on one of the session's helper
/// threads is held instead, and the session prints it once it has given the
/// terminal back itself.
fn restore_terminals_on_panic() {
    let print = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if cli::hold_helper_panic(info.to_string()) {
            return;
        }
        cli::restore_terminals();
        print(info);
    }));
}

/// The concrete adapters this binary runs on: the user's own database,
/// desktop and process table.
struct LocalPorts;

impl CliPorts for LocalPorts {
    fn open_state(&self) -> Result<SqliteStore, ConfigError> {
        SqliteStore::open_default()
    }

    fn run_service(&self) -> Result<(), MusterError> {
        run_service()
    }

    fn run_holder(&self, spec: &Path) -> Result<(), MusterError> {
        Ok(run_holder(spec)?)
    }
}

impl ProjectPorts for LocalPorts {
    fn notifier(&self) -> Option<Box<dyn Notifier>> {
        Some(Box::new(DesktopNotifier::new()))
    }

    fn process_metrics(&self) -> Option<Box<dyn ProcessMetrics>> {
        Some(Box::new(SysinfoMetrics::default()))
    }
}

/// Runs this user's service until a client stops or replaces it, unless one
/// already runs, in which case it returns at once. Its own stderr goes to the
/// service log first, so a failure to start is recorded even when it was
/// spawned with no terminal. Every process runs under a holder launched from
/// this binary, with its logs under the state directory.
///
/// # Errors
/// Returns a [`MusterError`] when the state directory, database or service
/// cannot be set up.
fn run_service() -> Result<(), MusterError> {
    let state_dir = state_dir().ok_or(ServiceError::NoStateDir)?;
    prepare_state_dir(&state_dir)?;
    redirect_stderr(&protocol::log_path(&state_dir))?;
    let Some(singleton) = Singleton::acquire(&state_dir)? else {
        eprintln!("{SERVICE_ALREADY_RUNNING} (pid {})", process::id());
        return Ok(());
    };
    let store = SqliteStore::open_default()?;
    let binary = env::current_exe()?;
    let launcher = ExecHolderLauncher::builder().binary(binary.clone()).build();
    let runner = HolderRunner::new(Box::new(launcher), HolderDirs::under(&state_dir));
    let deps = ServiceDeps::builder()
        .version(APP_VERSION.to_string())
        .binary(binary)
        .holders(Arc::new(runner))
        .ports(Box::new(LocalPorts))
        .stacks(Box::new(store.clone()))
        .settings(Box::new(store))
        .listening(None)
        .build();
    Ok(ServiceRuntime::run(singleton, deps)?)
}

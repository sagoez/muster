use std::{env, panic, path::Path, process, sync::Arc};

use muster::{
    adapter::{
        cli::{self, CliPorts, McpLaunch},
        client::{RunningServiceNotifier, ServiceAccess, UserService},
        holder::{ExecHolderLauncher, HolderDirs, HolderRunner, run_holder},
        listening::OsListeningPorts,
        mcp::{McpDeps, Scope, connection_config, run_stdio},
        metrics::SysinfoMetrics,
        notifier::DesktopNotifier,
        open_files::OpenFilesLimit,
        path::home_dir,
        port_occupants::OsPortOccupants,
        private_dir::{prepare_state_dir, state_dir},
        pty::PortablePtyRunner,
        service::{
            ProjectPorts, ServiceDeps, ServiceError, ServiceRuntime, ServiceVersion, Singleton,
            protocol, redirect_stderr,
        },
        store::SqliteStore,
    },
    domain::{
        config::ConfigError,
        port::{LocalState, Notifier, PortOccupants},
    },
    error::MusterError,
};

/// What a service started beside a running one records before it exits, so
/// each start that lost the race is visible in the service log.
const SERVICE_ALREADY_RUNNING: &str =
    "another muster service already runs for this state directory; this one exits";

/// The adapters every command of this binary runs on.
static PORTS: LocalPorts = LocalPorts;

/// Entry point: parses the command line and runs it on the concrete
/// adapters wired here.
fn main() {
    restore_terminals_on_panic();
    let args = cli::parse_args(&PORTS);
    cli::run(args, &PORTS);
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
    fn open_state(&self) -> Result<Arc<dyn LocalState>, ConfigError> {
        Ok(Arc::new(SqliteStore::open_default()?))
    }

    fn existing_state(&self) -> Result<Option<Arc<dyn LocalState>>, ConfigError> {
        Ok(SqliteStore::open_default_existing()?
            .map(|store| -> Arc<dyn LocalState> { Arc::new(store) }))
    }

    fn service(&self) -> Arc<dyn ServiceAccess> {
        Arc::new(UserService)
    }

    fn serve_mcp(&self, launch: McpLaunch) -> Result<(), MusterError> {
        serve_mcp(launch)
    }

    fn mcp_config(&self, executable: &Path) -> String {
        connection_config(executable)
    }

    fn run_service(&self) -> Result<(), MusterError> {
        run_service()
    }

    fn run_holder(&self, spec: &Path) -> Result<(), MusterError> {
        Ok(run_holder(spec, &PortablePtyRunner)?)
    }
}

impl ProjectPorts for LocalPorts {
    fn notifier(&self) -> Option<Box<dyn Notifier>> {
        Some(Box::new(DesktopNotifier::new()))
    }

    fn port_occupants(&self) -> Option<Box<dyn PortOccupants>> {
        Some(Box::new(OsPortOccupants::default()))
    }
}

/// Serves muster to an agent over MCP on stdio, on this user's database and
/// service, until the agent disconnects.
///
/// # Errors
/// Returns a [`MusterError`] when the database cannot be opened or the
/// server fails.
fn serve_mcp(launch: McpLaunch) -> Result<(), MusterError> {
    let store = SqliteStore::open_default()?;
    let service: Arc<dyn ServiceAccess> = Arc::new(UserService);
    let deps = McpDeps::builder()
        .scope(
            Scope::builder()
                .cwd(launch.cwd().clone())
                .base(launch.base().clone())
                .home(home_dir())
                .build(),
        )
        .agent(launch.agent().clone())
        .service(service.clone())
        .projects(Arc::new(store.clone()))
        .coordination(Arc::new(store.clone()))
        .stacks(Arc::new(store.clone()))
        .settings(Arc::new(store.clone()))
        .edits(Arc::new(store))
        .notifier(Arc::new(RunningServiceNotifier::new(service)))
        .build();
    Ok(run_stdio(deps)?)
}

/// Runs this user's service until a client stops or replaces it, unless one
/// already runs, in which case it returns at once. Its own stderr goes to the
/// service log first, so a failure to start is recorded even when it was
/// spawned with no terminal. Every process runs under a holder launched from
/// this binary, with its logs under the state directory, and starts at the
/// open-files limit the service started with, read before `sysinfo` raises it.
///
/// # Errors
/// Returns a [`MusterError`] when the state directory, database or service
/// cannot be set up.
fn run_service() -> Result<(), MusterError> {
    let open_files = OpenFilesLimit::current();
    let state_dir = state_dir().map_err(ServiceError::from)?;
    prepare_state_dir(&state_dir)?;
    redirect_stderr(&protocol::log_path(&state_dir))?;
    let Some(singleton) = Singleton::acquire(&state_dir)? else {
        eprintln!("{SERVICE_ALREADY_RUNNING} (pid {})", process::id());
        return Ok(());
    };
    let store = SqliteStore::open_default()?;
    let binary = env::current_exe()?;
    let launcher = ExecHolderLauncher::builder()
        .binary(binary.clone())
        .open_files(open_files)
        .build();
    let runner = HolderRunner::new(Box::new(launcher), HolderDirs::under(&state_dir));
    let deps = ServiceDeps::builder()
        .version(ServiceVersion::current())
        .binary(binary)
        .holders(Arc::new(runner))
        .ports(Box::new(LocalPorts))
        .metrics(Some(Box::new(SysinfoMetrics::default())))
        .stacks(Box::new(store.clone()))
        .settings(Box::new(store.clone()))
        .runs(Arc::new(store.clone()))
        .raised(Arc::new(store.clone()))
        .edits(Box::new(store))
        .listening(Some(Box::new(OsListeningPorts::default())))
        .build();
    Ok(ServiceRuntime::run(singleton, deps)?)
}

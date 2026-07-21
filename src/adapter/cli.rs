use std::path::{Path, PathBuf};

use clap_complete::{ArgValueCandidates, CompletionCandidate};
use thiserror::Error;

use crate::{
    adapter::config::YamlProjectRegistry,
    constants::MUSTER_PROJECT_ENV,
    domain::{
        config::{ConfigError, ProcessSpec, WorkspaceConfig},
        port::ProjectRegistry,
        process::ProcessKind,
        value::{CommandLine, ProcessName, ProjectName},
    },
};

/// Shell used to run a captured command, matching how the PTY runner launches
/// processes so "runs now" behaves the same as "runs later under muster".
#[cfg(unix)]
const SHELL_PROGRAM: &str = "/bin/sh";
/// Flag passing the command string to [`SHELL_PROGRAM`].
#[cfg(unix)]
const SHELL_FLAG: &str = "-c";

/// `muster run`: register a command as a process in a project, then run it.
///
/// The command is added to the project's workspace file (so it persists and is
/// managed on the next launch) and then executed in place, exactly as if it had
/// been typed without the `muster run` prefix.
#[derive(clap::Args)]
pub struct RunArgs {
    /// Project to add the command to (a registered project name). Defaults to
    /// the current project when run inside a muster pane.
    #[arg(short, long, add = ArgValueCandidates::new(project_candidates))]
    project: Option<String>,
    /// Sidebar name for the process. Defaults to the command's first word.
    #[arg(short, long)]
    name: Option<String>,
    /// How the process is grouped and managed.
    #[arg(short, long, value_enum, default_value_t = ProcessKindArg::Command)]
    kind: ProcessKindArg,
    /// The command to register and run, e.g. `-- npm run dev`.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
    command: Vec<String>,
}

/// The process kind, as a CLI-facing value enum.
#[derive(Clone, Copy, clap::ValueEnum)]
enum ProcessKindArg {
    Agent,
    Terminal,
    Command,
}

impl From<ProcessKindArg> for ProcessKind {
    fn from(kind: ProcessKindArg) -> Self {
        match kind {
            ProcessKindArg::Agent => Self::Agent,
            ProcessKindArg::Terminal => Self::Terminal,
            ProcessKindArg::Command => Self::Command,
        }
    }
}

/// A failure while capturing or running a command.
#[derive(Debug, Error)]
pub enum CliError {
    /// No registered project matched the requested name.
    #[error("unknown project '{0}'")]
    UnknownProject(String),
    /// The derived or given process name was empty.
    #[error("'{0}' is not a valid process name")]
    InvalidName(String),
    /// The command was blank.
    #[error("the command is empty")]
    EmptyCommand,
    /// The command could not be reassembled into a shell string.
    #[error("the command cannot be represented as a shell command")]
    InvalidCommand,
    /// `muster run` is not available on this platform.
    #[error("muster run is only supported on Unix")]
    Unsupported,
    /// The workspace file could not be read or written.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// The command could not be executed.
    #[error("failed to run the command: {0}")]
    Exec(#[source] std::io::Error),
}

/// Registers the command in the resolved project, then runs it in place. On
/// success this replaces the current process, so it does not return; any error
/// short-circuits before the command runs.
///
/// # Errors
/// Returns [`CliError`] if the project cannot be resolved, the workspace file
/// cannot be updated, or the command fails to start.
pub fn run(args: RunArgs, config: PathBuf, registry: &dyn ProjectRegistry) -> Result<(), CliError> {
    // Running the command replaces this process in place, which only works on
    // Unix. Refuse up front so a failed run never leaves a persisted-but-unrun
    // entry in the config on other platforms.
    if cfg!(not(unix)) {
        return Err(CliError::Unsupported);
    }
    let config_path = resolve(registry, args.project.as_deref(), env_project(), config)?;
    let command = command_string(&args.command)?;
    let command_line = CommandLine::try_new(command).map_err(|_| CliError::EmptyCommand)?;
    let name = process_name(args.name.as_deref(), &args.command)?;
    // The command runs in the caller's directory now; record it so a later start
    // from the sidebar runs there too instead of muster's launch directory.
    let spec = ProcessSpec::builder()
        .name(name)
        .command(Some(command_line.clone()))
        .working_dir(std::env::current_dir().ok())
        .build();
    register(registry, &config_path, spec, args.kind.into())?;
    // Run the stored form verbatim, so the immediate run matches what muster
    // will run later.
    Err(exec(command_line.as_ref()))
}

/// Turns the parsed argument vector into the command string run by `sh -c`.
///
/// A single argument is already a shell expression the user quoted (e.g. `'npm
/// test && npm run build'`), so it is passed through verbatim to preserve
/// pipelines, redirects, `&&`, and variable expansion. Multiple arguments came
/// from the shell's own tokenization, so each is re-escaped to preserve its
/// boundary (e.g. `printf '%s\n' 'hello world'`).
fn command_string(command: &[String]) -> Result<String, CliError> {
    match command {
        [expression] => Ok(expression.clone()),
        _ => shlex::try_join(command.iter().map(String::as_str))
            .map_err(|_| CliError::InvalidCommand),
    }
}

/// Resolves the target workspace path: an explicit project name looked up in the
/// registry, else the current-project environment variable, else the top-level
/// `--config` path (so `muster --config X run ...` targets X).
fn resolve(
    registry: &dyn ProjectRegistry,
    project: Option<&str>,
    env: Option<PathBuf>,
    config: PathBuf,
) -> Result<PathBuf, CliError> {
    match project {
        Some(name) => resolve_named(registry, name),
        None => Ok(env.unwrap_or(config)),
    }
}

/// Looks up a registered project's config path by name.
fn resolve_named(registry: &dyn ProjectRegistry, name: &str) -> Result<PathBuf, CliError> {
    let wanted =
        ProjectName::try_new(name).map_err(|_| CliError::UnknownProject(name.to_string()))?;
    registry
        .projects()?
        .into_iter()
        .find(|project| project.name().as_ref() == wanted.as_ref())
        .map(|project| project.config().clone())
        .ok_or_else(|| CliError::UnknownProject(name.to_string()))
}

/// Shell-completion candidates for `--project`: the names of registered
/// projects, read fresh each time so newly-created projects complete at once.
fn project_candidates() -> Vec<CompletionCandidate> {
    YamlProjectRegistry
        .projects()
        .unwrap_or_default()
        .iter()
        .map(|project| CompletionCandidate::new(project.name().as_ref()))
        .collect()
}

/// The current-project path from the environment, if muster exported one.
fn env_project() -> Option<PathBuf> {
    std::env::var_os(MUSTER_PROJECT_ENV)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// The process name to record: an explicit non-blank name, else the command's
/// first word.
fn process_name(explicit: Option<&str>, command: &[String]) -> Result<ProcessName, CliError> {
    let candidate = explicit
        .map(str::trim)
        .filter(|value| !value.is_empty())
        // The first word of the first argument: for a single quoted expression
        // like `npm test && npm run build` this is `npm`, not the whole string.
        .or_else(|| {
            command
                .first()
                .and_then(|first| first.split_whitespace().next())
        })
        .unwrap_or_default();
    ProcessName::try_new(candidate).map_err(|_| CliError::InvalidName(candidate.to_string()))
}

/// Appends `spec` to the section for `kind`, under the registry's locked
/// read-modify-write so concurrent `muster run` invocations do not clobber each
/// other.
fn register(
    registry: &dyn ProjectRegistry,
    config_path: &Path,
    spec: ProcessSpec,
    kind: ProcessKind,
) -> Result<(), CliError> {
    let mut append = |config: WorkspaceConfig| {
        let spec = spec.clone();
        match kind {
            ProcessKind::Agent => {
                let mut specs = config.agents().clone();
                specs.push(spec);
                config.with_agents(specs)
            },
            ProcessKind::Terminal => {
                let mut specs = config.terminals().clone();
                specs.push(spec);
                config.with_terminals(specs)
            },
            ProcessKind::Command => {
                let mut specs = config.commands().clone();
                specs.push(spec);
                config.with_commands(specs)
            },
        }
    };
    registry.update_workspace(config_path, &mut append)?;
    Ok(())
}

/// Replaces the current process with the command via the shell. Returns the
/// error only if the command could not be started (on success it never returns).
#[cfg(unix)]
fn exec(command: &str) -> CliError {
    use std::os::unix::process::CommandExt;

    let error = std::process::Command::new(SHELL_PROGRAM)
        .arg(SHELL_FLAG)
        .arg(command)
        .exec();
    CliError::Exec(error)
}

/// Unreachable on non-Unix: [`run`] refuses before persisting, so the platform's
/// missing in-place exec never reaches a shell.
#[cfg(not(unix))]
fn exec(_command: &str) -> CliError {
    CliError::Unsupported
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::domain::{config::WorkspaceConfig, project::Project};

    /// A registry with a fixed project list and workspace, recording any save.
    struct FakeRegistry {
        projects: Vec<Project>,
        workspace: WorkspaceConfig,
        saved: RefCell<Option<(PathBuf, WorkspaceConfig)>>,
    }

    impl ProjectRegistry for FakeRegistry {
        fn projects(&self) -> Result<Vec<Project>, ConfigError> {
            Ok(self.projects.clone())
        }

        fn workspace(&self, _config_path: &Path) -> Result<WorkspaceConfig, ConfigError> {
            Ok(self.workspace.clone())
        }

        fn workspace_exists(&self, _config_path: &Path) -> bool {
            false
        }

        fn save(&self, _projects: &[Project]) -> Result<(), ConfigError> {
            Ok(())
        }

        fn save_workspace(
            &self,
            config_path: &Path,
            config: &WorkspaceConfig,
        ) -> Result<(), ConfigError> {
            *self.saved.borrow_mut() = Some((config_path.to_path_buf(), config.clone()));
            Ok(())
        }
    }

    fn empty_workspace() -> WorkspaceConfig {
        WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![])
            .build()
    }

    fn registry_with(projects: Vec<Project>) -> FakeRegistry {
        FakeRegistry {
            projects,
            workspace: empty_workspace(),
            saved: RefCell::new(None),
        }
    }

    fn project(name: &str, config: &str) -> Project {
        Project::builder()
            .name(ProjectName::try_new(name).unwrap())
            .config(PathBuf::from(config))
            .build()
    }

    #[test]
    fn resolves_a_named_project_to_its_config_path() {
        let registry = registry_with(vec![
            project("web", "~/web/muster.yml"),
            project("api", "~/api/muster.yml"),
        ]);
        assert_eq!(
            resolve(&registry, Some("api"), None, PathBuf::from("muster.yml")).unwrap(),
            PathBuf::from("~/api/muster.yml")
        );
    }

    #[test]
    fn an_unknown_project_name_errors() {
        let registry = registry_with(vec![project("web", "~/web/muster.yml")]);
        assert!(matches!(
            resolve(&registry, Some("nope"), None, PathBuf::from("muster.yml")),
            Err(CliError::UnknownProject(_))
        ));
    }

    #[test]
    fn without_a_name_it_falls_back_to_environment_then_config() {
        let registry = registry_with(vec![]);
        assert_eq!(
            resolve(
                &registry,
                None,
                Some(PathBuf::from("/env/muster.yml")),
                PathBuf::from("/cfg/muster.yml"),
            )
            .unwrap(),
            PathBuf::from("/env/muster.yml"),
            "the environment path wins over --config"
        );
        assert_eq!(
            resolve(&registry, None, None, PathBuf::from("/cfg/muster.yml")).unwrap(),
            PathBuf::from("/cfg/muster.yml"),
            "with no name or environment, the --config path is the target"
        );
    }

    #[test]
    fn register_appends_the_command_to_its_section() {
        let registry = registry_with(vec![]);
        let spec = ProcessSpec::builder()
            .name(ProcessName::try_new("web").unwrap())
            .command(Some(CommandLine::try_new("npm run dev").unwrap()))
            .build();

        register(
            &registry,
            Path::new("/here/muster.yml"),
            spec,
            ProcessKind::Command,
        )
        .unwrap();

        let saved = registry.saved.borrow();
        let (path, config) = saved.as_ref().unwrap();
        assert_eq!(path, Path::new("/here/muster.yml"));
        assert_eq!(config.commands().len(), 1);
        assert_eq!(config.commands()[0].name().as_ref(), "web");
        assert!(
            config.agents().is_empty() && config.terminals().is_empty(),
            "only the command section grew"
        );
    }

    #[test]
    fn command_reconstruction_preserves_argument_boundaries() {
        // A plain-space join would flatten these; escaping must round-trip back
        // to the exact argument vector.
        let argv = vec![
            "printf".to_string(),
            "%s\n".to_string(),
            "hello world".to_string(),
        ];
        let rebuilt = command_string(&argv).unwrap();
        assert_eq!(
            shlex::split(&rebuilt).unwrap(),
            argv,
            "the escaped command re-splits into the original arguments"
        );
        assert_ne!(
            rebuilt,
            argv.join(" "),
            "escaping actually changed something"
        );
    }

    #[test]
    fn a_single_argument_is_kept_as_a_shell_expression() {
        // One quoted argument is a shell expression; it must reach `sh -c`
        // unescaped so `&&`, pipes, and redirects still work.
        let argv = vec!["npm test && npm run build".to_string()];
        assert_eq!(
            command_string(&argv).unwrap(),
            "npm test && npm run build",
            "a single argument passes through verbatim"
        );
    }

    #[test]
    fn the_process_name_defaults_to_the_first_word() {
        let command = vec!["npm".to_string(), "run".to_string(), "dev".to_string()];
        assert_eq!(
            process_name(None, &command).unwrap().as_ref(),
            "npm",
            "no explicit name uses the command's first word"
        );
        assert_eq!(process_name(Some("web"), &command).unwrap().as_ref(), "web");
        assert_eq!(
            process_name(Some("   "), &command).unwrap().as_ref(),
            "npm",
            "a blank explicit name falls back to the first word"
        );
        assert_eq!(
            process_name(None, &["npm test && npm run build".to_string()])
                .unwrap()
                .as_ref(),
            "npm",
            "a single quoted expression uses its first word, not the whole string"
        );
    }
}

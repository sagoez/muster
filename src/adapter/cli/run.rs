use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use clap_complete::{ArgValueCandidates, CompletionCandidate};
use getset::Getters;
use typed_builder::TypedBuilder;

use super::{
    action_report::ActionReport,
    error::CliError,
    process::{ready_to_send, start_registered, status},
    service_client::ServiceClient,
    settle_watch::SettleWatch,
};
use crate::{
    adapter::{
        config::YamlProjectRegistry,
        path::{absolutize, registered_config_path},
    },
    constants::{MUSTER_PROJECT_ENV, WORKSPACE_FILE_NAME},
    domain::{
        config::{ConfigError, ProcessSpec, WorkspaceConfig},
        port::ProjectRegistry,
        process::ProcessKind,
        project::Project,
        value::{CommandLine, ProcessName, ProjectName},
    },
};

/// Whether `muster up` starts a process `muster run` registers. Off: running
/// a command once does not enlist it in every later start of the stack.
const RUN_AUTOSTART: bool = false;

/// `muster run`: register a command as a process in a project, start it
/// under the muster service and attach to it.
///
/// The command is added to the project's workspace file, so it persists and
/// can be started again by name, then started like `muster start`.
#[derive(clap::Args)]
pub struct RunArgs {
    /// Project to add the command to (a registered project name). Defaults to
    /// the current project when run inside a process muster started.
    #[arg(short, long, add = ArgValueCandidates::new(project_candidates))]
    project: Option<String>,
    /// Name for the process. Defaults to the command's first word.
    #[arg(short, long)]
    name: Option<String>,
    /// Print the report as JSON.
    #[arg(long)]
    json: bool,
    /// How the process is grouped and managed.
    #[arg(short, long, value_enum, default_value_t = ProcessKindArg::Command)]
    kind: ProcessKindArg,
    /// The command to register and run, e.g. `-- npm run dev`.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
    command: Vec<String>,
}

/// A `muster run` checked and ready: the project to add to, and the process
/// to add.
#[derive(Debug, Clone, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct RunPlan {
    /// The project's config path.
    project: PathBuf,
    /// The process to add.
    spec: ProcessSpec,
    /// The section to add it to.
    kind: ProcessKind,
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

impl RunArgs {
    /// Whether the report prints as JSON.
    pub fn json(&self) -> bool {
        self.json
    }
}

impl RunPlan {
    /// Checks `args` and finds the project to add the command to: the
    /// registered project `--project` names, or the project every process
    /// command finds from `explicit_config`. Nothing is written yet.
    ///
    /// # Errors
    /// Returns [`CliError`] when the command or name is not usable, or the
    /// project cannot be found.
    pub fn prepare(
        args: &RunArgs,
        explicit_config: Option<&Path>,
        registry: &dyn ProjectRegistry,
    ) -> Result<Self, CliError> {
        let command = command_string(&args.command)?;
        let command_line = CommandLine::try_new(command).map_err(|_| CliError::EmptyCommand)?;
        let name = process_name(args.name.as_deref(), &args.command)?;
        let project = run_project(
            registry,
            args.project.as_deref(),
            explicit_config,
            current_project_from_env().as_deref(),
            Path::new(WORKSPACE_FILE_NAME),
        )?;
        if !project.exists() {
            return Err(CliError::NoProjectConfig(project));
        }
        if registry
            .workspace(&project)
            .is_ok_and(|config| config.uses_name(&name))
        {
            return Err(CliError::RunNameTaken { name, project });
        }
        // The service starts processes in their recorded working directory,
        // so record the caller's: the command then runs where it was typed.
        let spec = ProcessSpec::builder()
            .name(name)
            .command(Some(command_line))
            .working_dir(std::env::current_dir().ok())
            .autostart(RUN_AUTOSTART)
            .build();
        Ok(Self::builder()
            .project(project)
            .spec(spec)
            .kind(args.kind.into())
            .build())
    }

    /// Registers the command in its project, then starts it through `client`
    /// and reports it once it settled, telling `watch` meanwhile. A name an
    /// open agent session holds,
    /// which the config does not list, is refused first, and so is one a
    /// concurrent writer added to the config meanwhile.
    ///
    /// # Errors
    /// Returns [`CliError::RunNameTaken`] when a process already has the
    /// name, or another [`CliError`] when the workspace file cannot be
    /// updated or the service cannot start the command.
    pub fn carry_out(
        self,
        registry: &dyn ProjectRegistry,
        client: &ServiceClient,
        watch: &mut dyn SettleWatch,
    ) -> Result<ActionReport, CliError> {
        let name = self.spec.name().clone();
        let key = name.key();
        let taken = || CliError::RunNameTaken {
            name: name.clone(),
            project: self.project.clone(),
        };
        if status(client, &self.project)?
            .processes()
            .iter()
            .any(|process| process.name().key() == key)
        {
            return Err(taken());
        }
        ready_to_send(watch)?;
        register_unique(registry, &self.project, self.spec.clone(), self.kind)?;
        start_registered(client, &self.project, name, watch)
    }
}

/// Registers `spec` as [`register`] does, refusing with
/// [`CliError::RunNameTaken`] when the locked write found its name taken by
/// a concurrent writer; a duplicate of another name already in the file is
/// the config's own error.
fn register_unique(
    registry: &dyn ProjectRegistry,
    config_path: &Path,
    spec: ProcessSpec,
    kind: ProcessKind,
) -> Result<(), CliError> {
    let name = spec.name().clone();
    match register(registry, config_path, spec, kind) {
        Err(CliError::Config(ConfigError::DuplicateProcessName {
            name: duplicate, ..
        })) if duplicate.key() == name.key() => Err(CliError::RunNameTaken {
            name,
            project: config_path.to_path_buf(),
        }),
        registered => registered,
    }
}

/// Resolves the absolute path of the project a process command acts on:
/// the explicit `--config`, then the current project exported into
/// processes, then `./muster.yml`, then the first registered project.
///
/// # Errors
/// Returns [`ConfigError`] when the registry cannot be read or its fallback
/// project has an ambiguous legacy relative path.
pub fn resolve_project_config(
    explicit: Option<&Path>,
    current_project: Option<&Path>,
    local_config: &Path,
    registry: &dyn ProjectRegistry,
) -> Result<PathBuf, ConfigError> {
    if let Some(path) = explicit.or(current_project) {
        return Ok(absolutize(path));
    }
    if should_select_local_config(local_config) {
        return Ok(absolutize(local_config));
    }
    let projects = registry.projects()?;
    match projects.first() {
        Some(project) => registered_config_path(project),
        None => Ok(absolutize(local_config)),
    }
}

/// Whether local discovery should select this config entry. Errors other than a
/// missing entry stay local so an unreadable path cannot launch another project.
fn should_select_local_config(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(error) => error.kind() != ErrorKind::NotFound,
    }
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

/// Resolves the project `muster run` adds to: the registered project
/// `named` when one is named, otherwise the chain every process command
/// uses from `explicit`, `current_project` and `local_config`.
///
/// # Errors
/// Returns [`CliError`] when the named project is unknown or ambiguous, or
/// the registry cannot be read.
fn run_project(
    registry: &dyn ProjectRegistry,
    named: Option<&str>,
    explicit: Option<&Path>,
    current_project: Option<&Path>,
    local_config: &Path,
) -> Result<PathBuf, CliError> {
    match named {
        Some(name) => resolve_named(registry, name),
        None => Ok(resolve_project_config(
            explicit,
            current_project,
            local_config,
            registry,
        )?),
    }
}

/// Looks up a registered project's config path by name.
fn resolve_named(registry: &dyn ProjectRegistry, name: &str) -> Result<PathBuf, CliError> {
    let wanted =
        ProjectName::try_new(name).map_err(|_| CliError::UnknownProject(name.to_string()))?;
    let matches: Vec<Project> = registry
        .projects()?
        .into_iter()
        .filter(|project| project.name().as_ref() == wanted.as_ref())
        .collect();
    // Mirror `projects remove`: a duplicated name must never silently pick
    // one workspace to mutate.
    if matches.len() > 1 {
        return Err(CliError::AmbiguousProject {
            name: name.to_string(),
            count: matches.len(),
            paths: matches
                .iter()
                .map(|project| project.config().display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        });
    }
    let project = matches
        .into_iter()
        .next()
        .ok_or_else(|| CliError::UnknownProject(name.to_string()))?;
    Ok(registered_config_path(&project)?)
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

/// Returns the current-project path exported into child panes, if present.
pub fn current_project_from_env() -> Option<PathBuf> {
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
            run_project(&registry, Some("api"), None, None, Path::new("muster.yml")).unwrap(),
            absolutize(Path::new("~/api/muster.yml"))
        );
    }

    #[test]
    fn a_duplicated_project_name_is_ambiguous() {
        let registry = registry_with(vec![
            project("web", "~/site/muster.yml"),
            project("web", "~/app/muster.yml"),
        ]);
        assert!(
            matches!(
                run_project(&registry, Some("web"), None, None, Path::new("muster.yml")),
                Err(CliError::AmbiguousProject { count: 2, .. })
            ),
            "run -p must refuse to pick one of two same-named projects"
        );
    }

    #[test]
    fn an_unknown_project_name_errors() {
        let registry = registry_with(vec![project("web", "~/web/muster.yml")]);
        assert!(matches!(
            run_project(&registry, Some("nope"), None, None, Path::new("muster.yml")),
            Err(CliError::UnknownProject(_))
        ));
    }

    /// Without `--project`, `muster run` finds its project with the same
    /// chain as every process command: `--config`, then the environment,
    /// then the local file, then the first registered project.
    #[test]
    fn without_a_name_it_uses_the_shared_chain() {
        let first = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("first.yml");
        let registry = registry_with(vec![
            Project::builder()
                .name(ProjectName::try_new("first").unwrap())
                .config(first.clone())
                .build(),
        ]);
        let missing = Path::new("missing-local.yml");

        assert_eq!(
            run_project(
                &registry,
                None,
                Some(Path::new("/cfg/muster.yml")),
                Some(Path::new("/env/muster.yml")),
                missing,
            )
            .unwrap(),
            PathBuf::from("/cfg/muster.yml"),
            "--config wins over the environment, as for every other command"
        );
        assert_eq!(
            run_project(
                &registry,
                None,
                None,
                Some(Path::new("/env/muster.yml")),
                missing
            )
            .unwrap(),
            PathBuf::from("/env/muster.yml")
        );
        assert_eq!(
            run_project(&registry, None, None, None, missing).unwrap(),
            first,
            "with nothing else, the first registered project"
        );
    }

    /// Verifies explicit and pane-context paths outrank filesystem discovery.
    #[test]
    fn project_config_prefers_explicit_then_current_project() {
        let registry = registry_with(vec![project("registered", "registered.yml")]);
        let local = Path::new("missing-local.yml");

        assert_eq!(
            resolve_project_config(
                Some(Path::new("explicit.yml")),
                Some(Path::new("current.yml")),
                local,
                &registry,
            )
            .unwrap(),
            absolutize(Path::new("explicit.yml"))
        );
        assert_eq!(
            resolve_project_config(None, Some(Path::new("current.yml")), local, &registry).unwrap(),
            absolutize(Path::new("current.yml"))
        );
    }

    /// Verifies workspace resolution makes a symlink path absolute without
    /// replacing the selected alias with its target.
    #[test]
    fn project_config_preserves_an_explicit_symlink_path() {
        use std::{fs, os::unix::fs::symlink};

        let dir = std::env::temp_dir().join(format!("muster-cli-link-{}", std::process::id()));
        let target = dir.join("shared.yml");
        let link = dir.join("muster.yml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&target, "").unwrap();
        symlink(&target, &link).unwrap();
        let registry = registry_with(Vec::new());

        let resolved =
            resolve_project_config(Some(&link), None, Path::new("unused.yml"), &registry).unwrap();

        assert_eq!(resolved, link);
        fs::remove_dir_all(dir).unwrap();
    }

    /// Verifies a dangling local config alias is selected and left for the
    /// loader to report instead of falling through to a registered workspace.
    #[test]
    fn project_config_prefers_a_dangling_local_symlink() {
        use std::os::unix::fs::symlink;

        let dir = std::env::temp_dir().join(format!("muster-cli-dangling-{}", std::process::id()));
        let local = dir.join("muster.yml");
        fs::create_dir_all(&dir).unwrap();
        symlink(dir.join("missing.yml"), &local).unwrap();
        let registry = registry_with(vec![project("registered", "/other/muster.yml")]);

        let resolved = resolve_project_config(None, None, &local, &registry).unwrap();

        assert_eq!(resolved, local);
        fs::remove_dir_all(dir).unwrap();
    }

    /// Verifies an existing local workspace preserves the original bare-launch
    /// behavior even when global projects are registered.
    #[test]
    fn project_config_prefers_an_existing_local_workspace() {
        let registry = registry_with(vec![project("registered", "registered.yml")]);
        let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("muster.yml");

        assert_eq!(
            resolve_project_config(None, None, &local, &registry).unwrap(),
            local
        );
    }

    /// Verifies a bare launch outside a workspace uses the first global project.
    #[test]
    fn project_config_falls_back_to_the_global_registry() {
        let first = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("first.yml");
        let registry = registry_with(vec![
            Project::builder()
                .name(ProjectName::try_new("first").unwrap())
                .config(first.clone())
                .build(),
        ]);

        assert_eq!(
            resolve_project_config(None, None, Path::new("missing-local.yml"), &registry).unwrap(),
            first
        );
    }

    /// Verifies an ambiguous legacy registry entry fails closed rather than
    /// resolving against the directory of a later launch.
    #[test]
    fn project_config_rejects_a_legacy_relative_registry_path() {
        let registry = registry_with(vec![project("legacy", "muster.yml")]);

        let error = resolve_project_config(None, None, Path::new("missing-local.yml"), &registry)
            .unwrap_err();

        match error {
            ConfigError::RelativeProjectConfig { name, path } => {
                assert_eq!(name.as_ref(), "legacy");
                assert_eq!(path, PathBuf::from("muster.yml"));
            },
            other => panic!("unexpected error: {other}"),
        }
    }

    /// Verifies an empty registry retains the conventional path so its existing
    /// load error remains actionable.
    #[test]
    fn project_config_keeps_the_local_path_when_the_registry_is_empty() {
        let registry = registry_with(vec![]);
        let local = Path::new("missing-local.yml");

        assert_eq!(
            resolve_project_config(None, None, local, &registry).unwrap(),
            absolutize(local)
        );
    }

    #[test]
    fn register_appends_the_command_to_its_section() {
        let registry = registry_with(vec![]);
        let spec = ProcessSpec::builder()
            .name(ProcessName::try_new("web").unwrap())
            .command(Some(CommandLine::try_new("npm run dev").unwrap()))
            .autostart(false)
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

    /// A registry whose locked write refuses, as a concurrent writer's
    /// duplicate of `duplicate` would make it.
    struct RefusingRegistry {
        duplicate: &'static str,
    }

    impl ProjectRegistry for RefusingRegistry {
        fn projects(&self) -> Result<Vec<Project>, ConfigError> {
            Ok(Vec::new())
        }

        fn workspace(&self, _config_path: &Path) -> Result<WorkspaceConfig, ConfigError> {
            Ok(empty_workspace())
        }

        fn workspace_exists(&self, _config_path: &Path) -> bool {
            true
        }

        fn save(&self, _projects: &[Project]) -> Result<(), ConfigError> {
            Ok(())
        }

        fn save_workspace(
            &self,
            _config_path: &Path,
            _config: &WorkspaceConfig,
        ) -> Result<(), ConfigError> {
            Ok(())
        }

        fn update_workspace(
            &self,
            _config_path: &Path,
            _update: &mut dyn FnMut(WorkspaceConfig) -> WorkspaceConfig,
        ) -> Result<(), ConfigError> {
            Err(ConfigError::DuplicateProcessName {
                name: ProcessName::try_new(self.duplicate).unwrap(),
                first_kind: ProcessKind::Command,
                first_entry: 1,
                second_kind: ProcessKind::Command,
                second_entry: 2,
            })
        }
    }

    /// The locked write refusing the name being added reports it as taken;
    /// a duplicate of another name is the config's own error.
    #[test]
    fn only_the_name_being_added_is_reported_as_taken() {
        let project = Path::new("/p/muster.yml");
        let spec = || {
            ProcessSpec::builder()
                .name(ProcessName::try_new("api").unwrap())
                .command(Some(CommandLine::try_new("sleep 600").unwrap()))
                .working_dir(None)
                .autostart(RUN_AUTOSTART)
                .build()
        };

        let same = RefusingRegistry { duplicate: "api" };
        assert!(matches!(
            register_unique(&same, project, spec(), ProcessKind::Command),
            Err(CliError::RunNameTaken { .. })
        ));
        let other = RefusingRegistry {
            duplicate: "worker",
        };
        assert!(matches!(
            register_unique(&other, project, spec(), ProcessKind::Command),
            Err(CliError::Config(ConfigError::DuplicateProcessName { .. }))
        ));
    }
}

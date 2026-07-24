use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{
    error::CliError,
    report::{Row, RowKind},
};
use crate::{
    adapter::path::absolutize,
    constants::WORKSPACE_FILE_NAME,
    domain::{port::ProjectRegistry, project::Project},
};

/// Marker prefixing the project that contains the current directory.
const CURRENT_MARKER: &str = "* ";
/// Indent for projects that are not current, aligning the columns.
const OTHER_INDENT: &str = "  ";
/// Line printed when no projects are registered.
const EMPTY_NOTE: &str = "no registered projects; run `muster init` in a project folder";

/// Lists registered projects, marking the one containing `current_dir`.
///
/// # Errors
/// Returns [`CliError`] when the registry cannot be read.
pub fn list(registry: &dyn ProjectRegistry, current_dir: &Path) -> Result<Vec<Row>, CliError> {
    let projects = registry.projects()?;
    if projects.is_empty() {
        return Ok(vec![Row::unlabeled(RowKind::Hint, EMPTY_NOTE)]);
    }
    Ok(projects
        .iter()
        .map(|project| {
            let marker = if is_current(project, current_dir) {
                CURRENT_MARKER
            } else {
                OTHER_INDENT
            };
            let folder = absolutize(project.config())
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| project.config().display().to_string());
            Row::unlabeled(
                RowKind::Plain,
                format!("{marker}{}  {folder}", project.name().as_ref()),
            )
        })
        .collect())
}

/// Whether the project's folder contains the current directory. Both sides
/// compare by canonical identity: the registry may hold a symlink alias while
/// the OS reports the resolved working directory.
fn is_current(project: &Project, current_dir: &Path) -> bool {
    absolutize(project.config())
        .parent()
        .is_some_and(|folder| resolved_identity(current_dir).starts_with(resolved_identity(folder)))
}

/// The canonical form of a path when it exists, else the path itself.
fn resolved_identity(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Registers an existing folder that already contains a workspace file.
///
/// # Errors
/// Returns [`CliError::MissingWorkspaceFile`] when the folder has no
/// `muster.yml`, when the path is a directory, or when it is a dangling
/// symlink - anything other than a readable regular file.
pub fn add(directory: &Path, registry: &dyn ProjectRegistry) -> Result<Vec<Row>, CliError> {
    let config_path = absolutize(&directory.join(WORKSPACE_FILE_NAME));
    if !fs::metadata(&config_path)
        .map(|meta| meta.is_file())
        .unwrap_or(false)
    {
        return Err(CliError::MissingWorkspaceFile(config_path));
    }
    let name = super::init::project_name(directory)?;
    super::init::ensure_representable(&config_path)?;
    Ok(vec![super::init::register_folder(
        name,
        &config_path,
        registry,
    )?])
}

/// Outcome captured inside the `update_projects` closure.
enum RemoveOutcome {
    Removed,
    Unknown { known: String },
    Ambiguous { count: usize, paths: String },
}

/// Unregisters the named project; files on disk are untouched.
///
/// The match-count check and removal run inside `update_projects` so the read
/// and the write are atomic with respect to concurrent registry mutations.
///
/// # Errors
/// Returns [`CliError::UnknownProjectAmong`] when no project has that name, or
/// [`CliError::AmbiguousProject`] when more than one project shares the name.
pub fn remove(name: &str, registry: &dyn ProjectRegistry) -> Result<Vec<Row>, CliError> {
    let mut outcome = RemoveOutcome::Removed;

    registry.update_projects(&mut |projects| {
        let matches: Vec<&Project> = projects
            .iter()
            .filter(|p| p.name().as_ref() == name)
            .collect();

        if matches.is_empty() {
            outcome = RemoveOutcome::Unknown {
                known: projects
                    .iter()
                    .map(|p| p.name().as_ref().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            };
            return projects;
        }

        if matches.len() > 1 {
            let paths = matches
                .iter()
                .map(|p| p.config().display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            outcome = RemoveOutcome::Ambiguous {
                count: matches.len(),
                paths,
            };
            return projects;
        }

        projects
            .into_iter()
            .filter(|p| p.name().as_ref() != name)
            .collect()
    })?;

    match outcome {
        RemoveOutcome::Removed => Ok(vec![Row::unlabeled(
            RowKind::Ok,
            format!("removed '{name}' from the registry"),
        )]),
        RemoveOutcome::Unknown { known } => Err(CliError::UnknownProjectAmong {
            name: name.to_string(),
            known,
        }),
        RemoveOutcome::Ambiguous { count, paths } => Err(CliError::AmbiguousProject {
            name: name.to_string(),
            count,
            paths,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, fs, path::PathBuf};

    use super::*;
    use crate::domain::{
        config::{ConfigError, WorkspaceConfig},
        project::Project,
        value::ProjectName,
    };

    /// A registry recording saves of projects and workspaces.
    #[derive(Default)]
    struct RecordingRegistry {
        projects: Vec<Project>,
        saved_projects: RefCell<Option<Vec<Project>>>,
    }

    impl ProjectRegistry for RecordingRegistry {
        fn projects(&self) -> Result<Vec<Project>, ConfigError> {
            Ok(self.projects.clone())
        }

        fn workspace(&self, _config_path: &Path) -> Result<WorkspaceConfig, ConfigError> {
            unreachable!("projects never loads a workspace")
        }

        fn workspace_exists(&self, _config_path: &Path) -> bool {
            false
        }

        fn save(&self, projects: &[Project]) -> Result<(), ConfigError> {
            *self.saved_projects.borrow_mut() = Some(projects.to_vec());
            Ok(())
        }

        fn save_workspace(
            &self,
            _config_path: &Path,
            _config: &WorkspaceConfig,
        ) -> Result<(), ConfigError> {
            unreachable!("projects never saves a workspace")
        }
    }

    fn project(name: &str, config: &str) -> Project {
        Project::builder()
            .name(ProjectName::try_new(name).unwrap())
            .config(PathBuf::from(config))
            .build()
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("muster-projects-{tag}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Listing marks the project containing the current directory.
    #[test]
    fn list_marks_the_current_project() {
        let registry = RecordingRegistry {
            projects: vec![
                project("web", "/w/web/muster.yml"),
                project("api", "/w/api/muster.yml"),
            ],
            ..RecordingRegistry::default()
        };
        let rows = list(&registry, Path::new("/w/api/src")).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(!rows[0].detail().starts_with(CURRENT_MARKER));
        assert!(rows[1].detail().starts_with(CURRENT_MARKER));
        assert!(rows[1].detail().contains("api") && rows[1].detail().contains("/w/api"));
        // The second column must be the project folder, not the config file path.
        assert!(!rows[1].detail().contains("muster.yml"));
    }

    /// A project registered through a symlink alias still gets the current
    /// marker when the working directory is its resolved target.
    #[cfg(unix)]
    #[test]
    fn list_marks_a_symlinked_current_project() {
        use std::os::unix::fs::symlink;

        let base = temp_dir("symlink-current");
        let real = base.join("real");
        let link = base.join("link");
        fs::create_dir_all(real.join("src")).unwrap();
        symlink(&real, &link).unwrap();
        let registry = RecordingRegistry {
            projects: vec![
                Project::builder()
                    .name(ProjectName::try_new("here").unwrap())
                    .config(link.join(WORKSPACE_FILE_NAME))
                    .build(),
            ],
            ..RecordingRegistry::default()
        };

        let rows = list(&registry, &real.join("src")).unwrap();

        assert!(
            rows[0].detail().starts_with(CURRENT_MARKER),
            "resolved cwd matches the aliased registration: {}",
            rows[0].detail()
        );
        fs::remove_dir_all(base).unwrap();
    }

    /// An empty registry says so instead of printing nothing.
    #[test]
    fn list_reports_an_empty_registry() {
        let registry = RecordingRegistry::default();
        let rows = list(&registry, Path::new("/")).unwrap();
        assert_eq!(rows[0].detail(), EMPTY_NOTE);
    }

    /// Adding requires the folder to contain a workspace file.
    #[test]
    fn add_requires_a_workspace_file() {
        let dir = temp_dir("no-config");
        let registry = RecordingRegistry::default();
        assert!(matches!(
            add(&dir, &registry),
            Err(CliError::MissingWorkspaceFile(_))
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    /// A directory named muster.yml is not a valid workspace file.
    #[test]
    fn add_rejects_a_directory_named_muster_yml() {
        let dir = temp_dir("dir-as-config");
        fs::create_dir(dir.join(WORKSPACE_FILE_NAME)).unwrap();
        let registry = RecordingRegistry::default();
        assert!(matches!(
            add(&dir, &registry),
            Err(CliError::MissingWorkspaceFile(_))
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    /// Adding a folder with a config registers it once.
    #[test]
    fn add_registers_and_readd_is_a_no_op() {
        let dir = temp_dir("add");
        fs::write(dir.join(WORKSPACE_FILE_NAME), "agents: []\n").unwrap();
        let registry = RecordingRegistry::default();

        let rows = add(&dir, &registry).unwrap();
        assert!(registry.saved_projects.borrow().is_some());
        assert!(rows[0].detail().contains("registered"));

        let config = crate::adapter::path::absolutize(&dir.join(WORKSPACE_FILE_NAME));
        let seeded = RecordingRegistry {
            projects: vec![
                Project::builder()
                    .name(ProjectName::try_new("here").unwrap())
                    .config(config)
                    .build(),
            ],
            ..RecordingRegistry::default()
        };
        let again = add(&dir, &seeded).unwrap();
        // The list is written back unchanged: still exactly the seeded entry.
        {
            let saved = seeded.saved_projects.borrow();
            let written = saved.as_ref().expect("list written back");
            assert_eq!(written.len(), 1, "re-add keeps exactly one project");
            assert_eq!(written[0].name().as_ref(), "here", "the seeded entry kept");
        }
        assert!(again[0].detail().contains("already registered"));

        fs::remove_dir_all(dir).unwrap();
    }

    /// The same folder addressed through parent components is recognized as
    /// already registered instead of appended twice.
    #[test]
    fn add_normalizes_parent_components() {
        let dir = temp_dir("dotdot");
        fs::write(dir.join(WORKSPACE_FILE_NAME), "agents: []\n").unwrap();
        fs::create_dir_all(dir.join("sub")).unwrap();
        let config = crate::adapter::path::absolutize(&dir.join(WORKSPACE_FILE_NAME));
        let seeded = RecordingRegistry {
            projects: vec![
                Project::builder()
                    .name(ProjectName::try_new("here").unwrap())
                    .config(config)
                    .build(),
            ],
            ..RecordingRegistry::default()
        };

        let aliased = dir.join("sub").join("..");
        let rows = add(&aliased, &seeded).unwrap();

        let saved = seeded.saved_projects.borrow();
        let written = saved.as_ref().expect("list written back");
        assert_eq!(written.len(), 1, "no duplicate entry for the same folder");
        assert!(rows[0].detail().contains("already registered"));
        fs::remove_dir_all(dir).unwrap();
    }

    /// Removing an unknown name lists the known ones.
    #[test]
    fn remove_unknown_lists_known_names() {
        let registry = RecordingRegistry {
            projects: vec![project("web", "/w/web/muster.yml")],
            ..RecordingRegistry::default()
        };
        match remove("nope", &registry) {
            Err(CliError::UnknownProjectAmong { name, known }) => {
                assert_eq!(name, "nope");
                assert!(known.contains("web"));
            },
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// Removing a name that matches two projects returns AmbiguousProject and
    /// leaves the list unchanged.
    #[test]
    fn remove_rejects_ambiguous_name() {
        let registry = RecordingRegistry {
            projects: vec![
                project("web", "/w/site/muster.yml"),
                project("web", "/w/app/muster.yml"),
            ],
            ..RecordingRegistry::default()
        };
        match remove("web", &registry) {
            Err(CliError::AmbiguousProject { name, count, paths }) => {
                assert_eq!(name, "web");
                assert_eq!(count, 2);
                assert!(paths.contains("/w/site/muster.yml"));
                assert!(paths.contains("/w/app/muster.yml"));
            },
            other => panic!("unexpected: {other:?}"),
        }
        // The list is written back unchanged: both same-named entries survive
        // with their distinct paths.
        let saved = registry.saved_projects.borrow();
        let written = saved.as_ref().expect("list written back");
        assert_eq!(written.len(), 2, "both projects remain");
        assert!(
            written
                .iter()
                .any(|project| project.config().ends_with("site/muster.yml"))
                && written
                    .iter()
                    .any(|project| project.config().ends_with("app/muster.yml")),
            "each original path survives"
        );
    }

    /// Removing keeps every other project and never touches files.
    #[test]
    fn remove_drops_only_the_named_project() {
        let registry = RecordingRegistry {
            projects: vec![
                project("web", "/w/web/muster.yml"),
                project("api", "/w/api/muster.yml"),
            ],
            ..RecordingRegistry::default()
        };
        remove("web", &registry).unwrap();
        let saved = registry.saved_projects.borrow();
        let names: Vec<_> = saved
            .as_ref()
            .unwrap()
            .iter()
            .map(|p| p.name().as_ref().to_string())
            .collect();
        assert_eq!(names, vec!["api"]);
    }
}

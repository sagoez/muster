use std::{fs, path::Path};

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

/// Whether the project's folder contains the current directory.
fn is_current(project: &Project, current_dir: &Path) -> bool {
    absolutize(project.config())
        .parent()
        .is_some_and(|folder| current_dir.starts_with(folder))
}

/// Registers an existing folder that already contains a workspace file.
///
/// # Errors
/// Returns [`CliError::MissingWorkspaceFile`] when the folder has no
/// `muster.yml`, or a registry error when it cannot be updated.
pub fn add(directory: &Path, registry: &dyn ProjectRegistry) -> Result<Vec<Row>, CliError> {
    let config_path = absolutize(&directory.join(WORKSPACE_FILE_NAME));
    if fs::symlink_metadata(&config_path).is_err() {
        return Err(CliError::MissingWorkspaceFile(config_path));
    }
    Ok(vec![super::init::register_folder(
        directory,
        &config_path,
        registry,
    )?])
}

/// Unregisters the named project; files on disk are untouched.
///
/// # Errors
/// Returns [`CliError::UnknownProjectAmong`] when no project has that name, or
/// [`CliError::AmbiguousProject`] when more than one project shares the name.
pub fn remove(name: &str, registry: &dyn ProjectRegistry) -> Result<Vec<Row>, CliError> {
    let projects = registry.projects()?;
    let matches: Vec<&Project> = projects
        .iter()
        .filter(|project| project.name().as_ref() == name)
        .collect();
    if matches.is_empty() {
        return Err(CliError::UnknownProjectAmong {
            name: name.to_string(),
            known: projects
                .iter()
                .map(|project| project.name().as_ref().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        });
    }
    if matches.len() > 1 {
        let paths = matches
            .iter()
            .map(|project| project.config().display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(CliError::AmbiguousProject {
            name: name.to_string(),
            count: matches.len(),
            paths,
        });
    }
    let remaining: Vec<Project> = projects
        .iter()
        .filter(|project| project.name().as_ref() != name)
        .cloned()
        .collect();
    registry.save(&remaining)?;
    Ok(vec![Row::unlabeled(
        RowKind::Ok,
        format!("removed '{name}' from the registry"),
    )])
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
        assert!(
            seeded.saved_projects.borrow().is_none(),
            "re-add saves nothing"
        );
        assert!(again[0].detail().contains("already registered"));

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
    /// saves nothing.
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
        assert!(
            registry.saved_projects.borrow().is_none(),
            "ambiguous remove saves nothing"
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

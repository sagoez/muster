use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{
    error::CliError,
    report::{Row, RowKind},
};
use crate::{
    adapter::{config::starter_workspace, path::absolutize},
    constants::WORKSPACE_FILE_NAME,
    domain::{port::ProjectRegistry, project::Project, value::ProjectName},
};

/// Note printed when the workspace file already exists.
const EXISTS_NOTE: &str = "already exists, left unchanged";
/// Note printed when the folder is already a registered project.
const REGISTERED_NOTE: &str = "already registered";
/// Closing hint after a successful init.
const RUN_HINT: &str = "run `muster` here to start";

/// Scaffolds a starter workspace in `directory` and registers it as a project.
/// Returns the report rows to print, in order.
///
/// # Errors
/// Returns [`CliError`] when the folder name is not a usable project name or
/// the registry cannot be read or written.
pub fn init(directory: &Path, registry: &dyn ProjectRegistry) -> Result<Vec<Row>, CliError> {
    let config_path = absolutize(&directory.join(WORKSPACE_FILE_NAME));
    let mut rows = Vec::new();
    // Use symlink_metadata so a dangling symlink is never overwritten -
    // intentionally asymmetric with `projects add` and `doctor`, which
    // require a reachable regular file.
    if fs::symlink_metadata(&config_path).is_ok() {
        rows.push(Row::unlabeled(
            RowKind::Hint,
            format!("{WORKSPACE_FILE_NAME} {EXISTS_NOTE}"),
        ));
    } else {
        registry.save_workspace(&config_path, &starter_workspace())?;
        rows.push(Row::unlabeled(
            RowKind::Ok,
            format!("created {WORKSPACE_FILE_NAME}"),
        ));
    }
    rows.push(register_folder(directory, &config_path, registry)?);
    rows.push(Row::unlabeled(RowKind::Dim, RUN_HINT));
    Ok(rows)
}

/// Registers the folder as a project unless its config path already is one.
/// `pub(super)` because `muster projects add` reuses it.
///
/// The check-and-insert runs inside `update_projects` so it is atomic with
/// respect to concurrent `muster init` or `muster projects add` invocations.
///
/// # Errors
/// Returns [`CliError`] when the folder has no usable name or the registry
/// cannot be updated.
pub(super) fn register_folder(
    directory: &Path,
    config_path: &Path,
    registry: &dyn ProjectRegistry,
) -> Result<Row, CliError> {
    // Derive the name before entering the lock - this may fail early.
    let name = project_name(directory)?;
    let label = name.as_ref().to_string();

    // Capture the already-registered project name (if any) inside the closure
    // so the outcome is observable after the lock releases.
    let mut existing_name: Option<String> = None;

    registry.update_projects(&mut |projects| {
        if let Some(existing) = projects
            .iter()
            .find(|project| absolutize(project.config()) == config_path)
        {
            existing_name = Some(existing.name().as_ref().to_string());
            // Already registered - return list unchanged.
            return projects;
        }
        let mut updated = projects;
        updated.push(
            Project::builder()
                .name(name.clone())
                .config(config_path.to_path_buf())
                .build(),
        );
        updated
    })?;

    if let Some(found) = existing_name {
        Ok(Row::unlabeled(
            RowKind::Hint,
            format!("{REGISTERED_NOTE} as '{found}'"),
        ))
    } else {
        Ok(Row::unlabeled(
            RowKind::Ok,
            format!("registered project '{label}'"),
        ))
    }
}

/// The project name derived from the folder's file name.
///
/// # Errors
/// Returns [`CliError::InvalidProjectFolder`] when the folder yields no name.
fn project_name(directory: &Path) -> Result<ProjectName, CliError> {
    let file_name = directory
        .canonicalize()
        .unwrap_or_else(|_| directory.to_path_buf())
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    ProjectName::try_new(&file_name)
        .map_err(|_| CliError::InvalidProjectFolder(PathBuf::from(directory)))
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
        saved_workspace: RefCell<Option<PathBuf>>,
    }

    impl ProjectRegistry for RecordingRegistry {
        fn projects(&self) -> Result<Vec<Project>, ConfigError> {
            Ok(self.projects.clone())
        }

        fn workspace(&self, _config_path: &Path) -> Result<WorkspaceConfig, ConfigError> {
            unreachable!("init never loads a workspace")
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
            config_path: &Path,
            _config: &WorkspaceConfig,
        ) -> Result<(), ConfigError> {
            *self.saved_workspace.borrow_mut() = Some(config_path.to_path_buf());
            Ok(())
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("muster-init-{tag}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A fresh folder gets a starter config and a registry entry.
    #[test]
    fn scaffolds_and_registers_a_fresh_folder() {
        let dir = temp_dir("fresh");
        let registry = RecordingRegistry::default();

        let rows = init(&dir, &registry).unwrap();

        assert!(
            registry.saved_workspace.borrow().is_some(),
            "config written"
        );
        let saved = registry.saved_projects.borrow();
        assert_eq!(saved.as_ref().unwrap().len(), 1, "project registered");
        assert!(
            rows.iter()
                .any(|row| row.detail().contains(WORKSPACE_FILE_NAME))
        );
        fs::remove_dir_all(dir).unwrap();
    }

    /// An existing config is never overwritten, but registration still runs.
    #[test]
    fn refuses_to_overwrite_but_still_registers() {
        let dir = temp_dir("existing");
        fs::write(dir.join(WORKSPACE_FILE_NAME), "agents: []\n").unwrap();
        let registry = RecordingRegistry::default();

        let rows = init(&dir, &registry).unwrap();

        assert!(registry.saved_workspace.borrow().is_none(), "no overwrite");
        assert!(
            registry.saved_projects.borrow().is_some(),
            "still registered"
        );
        assert!(rows.iter().any(|row| row.detail().contains(EXISTS_NOTE)));
        fs::remove_dir_all(dir).unwrap();
    }

    /// An already registered project is reported, not duplicated.
    #[test]
    fn re_init_is_a_registration_no_op() {
        let dir = temp_dir("registered");
        fs::write(dir.join(WORKSPACE_FILE_NAME), "agents: []\n").unwrap();
        let config = crate::adapter::path::absolutize(&dir.join(WORKSPACE_FILE_NAME));
        let registry = RecordingRegistry {
            projects: vec![
                Project::builder()
                    .name(ProjectName::try_new("here").unwrap())
                    .config(config)
                    .build(),
            ],
            ..RecordingRegistry::default()
        };

        let rows = init(&dir, &registry).unwrap();

        // The list is written back unchanged (identical content) but no new
        // project is appended.
        let saved = registry.saved_projects.borrow();
        assert_eq!(
            saved.as_ref().map(|v| v.len()),
            Some(1),
            "one project remains"
        );
        assert!(
            rows.iter()
                .any(|row| row.detail().contains(REGISTERED_NOTE))
        );
        fs::remove_dir_all(dir).unwrap();
    }

    /// `register_folder` routes through `update_projects`, not a bare
    /// `projects()` + `save()` pair, so the check-and-insert is atomic.
    #[test]
    fn register_folder_goes_through_update_projects() {
        /// Fake that records whether `update_projects` was called.
        #[derive(Default)]
        struct TrackingRegistry {
            update_projects_called: RefCell<bool>,
            saved_projects: RefCell<Option<Vec<Project>>>,
        }

        impl ProjectRegistry for TrackingRegistry {
            fn projects(&self) -> Result<Vec<Project>, ConfigError> {
                Ok(Vec::new())
            }

            fn workspace(&self, _config_path: &Path) -> Result<WorkspaceConfig, ConfigError> {
                unreachable!()
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
                unreachable!()
            }

            fn update_projects(
                &self,
                update: &mut dyn FnMut(Vec<Project>) -> Vec<Project>,
            ) -> Result<(), ConfigError> {
                *self.update_projects_called.borrow_mut() = true;
                // Delegate to default logic so the insertion actually runs.
                let projects = self.projects()?;
                self.save(&update(projects))
            }
        }

        let dir = temp_dir("atomic");
        let config_path = crate::adapter::path::absolutize(&dir.join(WORKSPACE_FILE_NAME));
        let registry = TrackingRegistry::default();

        register_folder(&dir, &config_path, &registry).unwrap();

        assert!(
            *registry.update_projects_called.borrow(),
            "register_folder must call update_projects"
        );
        assert!(
            registry.saved_projects.borrow().is_some(),
            "project was saved"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}

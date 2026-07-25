use std::path::{Path, PathBuf};

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
    // Validate everything the registry will persist first: a folder that
    // cannot become a project (a filesystem root, a non-UTF-8 path) must
    // fail before anything is written.
    let name = project_name(directory)?;
    let config_path = absolutize(&directory.join(WORKSPACE_FILE_NAME));
    ensure_representable(&config_path)?;
    let mut rows = Vec::new();
    // An exclusive create keeps the no-overwrite guarantee even against a
    // concurrent creation. Anything already occupying the path - including a
    // dangling symlink, which the exclusive open also refuses - is left
    // untouched, intentionally asymmetric with `projects add` and `doctor`,
    // which require a reachable regular file.
    if registry.create_workspace(&config_path, &starter_workspace())? {
        rows.push(Row::unlabeled(
            RowKind::Ok,
            format!("created {WORKSPACE_FILE_NAME}"),
        ));
    } else {
        rows.push(Row::unlabeled(
            RowKind::Hint,
            format!("{WORKSPACE_FILE_NAME} {EXISTS_NOTE}"),
        ));
    }
    rows.push(register_folder(name, &config_path, registry)?);
    rows.push(Row::unlabeled(RowKind::Dim, RUN_HINT));
    Ok(rows)
}

/// Registers `name` at `config_path` unless that config path already is a
/// project. `pub(super)` because `muster projects add` reuses it.
///
/// The check-and-insert runs inside `update_projects` so it is atomic with
/// respect to concurrent `muster init` or `muster projects add` invocations.
///
/// # Errors
/// Returns [`CliError`] when the registry cannot be updated.
pub(super) fn register_folder(
    name: ProjectName,
    config_path: &Path,
    registry: &dyn ProjectRegistry,
) -> Result<Row, CliError> {
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

/// The project name derived from the folder's file name. `pub(super)` because
/// `muster projects add` derives the same way.
///
/// # Errors
/// Returns [`CliError::InvalidProjectFolder`] when the folder yields no name.
pub(super) fn project_name(directory: &Path) -> Result<ProjectName, CliError> {
    // Normalize without resolving the final symlink: the registry preserves
    // the alias the user selected, so its name must come from the alias too.
    // No lossy conversion: a non-UTF-8 name would not round-trip through the
    // registry, so it is rejected rather than mangled.
    let file_name = absolutize(directory)
        .file_name()
        .and_then(|name| name.to_str().map(str::to_string))
        .unwrap_or_default();
    ProjectName::try_new(&file_name)
        .map_err(|_| CliError::InvalidProjectFolder(PathBuf::from(directory)))
}

/// Rejects a config path the registry cannot persist as UTF-8.
///
/// # Errors
/// Returns [`CliError::UnrepresentablePath`] for non-UTF-8 paths.
pub(super) fn ensure_representable(config_path: &Path) -> Result<(), CliError> {
    if config_path.to_str().is_none() {
        return Err(CliError::UnrepresentablePath(config_path.to_path_buf()));
    }
    Ok(())
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

        fn workspace_exists(&self, config_path: &Path) -> bool {
            config_path.exists()
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

    /// A symlinked folder keeps its alias name, matching the alias the
    /// registry stores, so two aliases of one target stay distinguishable.
    #[cfg(unix)]
    #[test]
    fn project_name_keeps_the_symlink_alias() {
        use std::os::unix::fs::symlink;

        let base = temp_dir("alias-name");
        let target = base.join("real-target");
        let alias = base.join("nice-alias");
        fs::create_dir_all(&target).unwrap();
        symlink(&target, &alias).unwrap();

        assert_eq!(project_name(&alias).unwrap().as_ref(), "nice-alias");
        fs::remove_dir_all(base).unwrap();
    }

    /// A non-UTF-8 folder fails before anything is written, not after the
    /// workspace exists. The directory is never created: APFS rejects
    /// non-UTF-8 names entirely, and init's validation must fire before any
    /// filesystem access anyway.
    #[cfg(unix)]
    #[test]
    fn init_rejects_a_non_utf8_folder_before_writing() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        let mut raw = std::env::temp_dir().into_os_string().into_vec();
        raw.extend_from_slice(b"/muster-bad-\xff");
        let dir = PathBuf::from(OsString::from_vec(raw));
        let registry = RecordingRegistry::default();

        let result = init(&dir, &registry);

        assert!(
            matches!(
                result,
                Err(CliError::InvalidProjectFolder(_) | CliError::UnrepresentablePath(_))
            ),
            "unexpected: {result:?}"
        );
        assert!(
            registry.saved_workspace.borrow().is_none(),
            "no workspace file was created"
        );
        assert!(
            registry.saved_projects.borrow().is_none(),
            "no registration"
        );
    }

    /// A directory with no usable name fails before anything is written.
    #[test]
    fn init_at_a_root_creates_nothing() {
        let registry = RecordingRegistry::default();

        let result = init(Path::new("/"), &registry);

        assert!(matches!(result, Err(CliError::InvalidProjectFolder(_))));
        assert!(
            registry.saved_workspace.borrow().is_none(),
            "no workspace file was created"
        );
        assert!(
            registry.saved_projects.borrow().is_none(),
            "no registration"
        );
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

        // The list is written back unchanged: still exactly the seeded entry.
        let saved = registry.saved_projects.borrow();
        let written = saved.as_ref().expect("list written back");
        assert_eq!(written.len(), 1, "one project remains");
        assert_eq!(written[0].name().as_ref(), "here", "the seeded entry kept");
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

        register_folder(project_name(&dir).unwrap(), &config_path, &registry).unwrap();

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

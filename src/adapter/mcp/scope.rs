//! Which project each tool call acts on, resolved per call through the rule
//! the CLI follows, so a folder registered after the server started is
//! found.

use std::path::{Path, PathBuf};

use getset::Getters;
use typed_builder::TypedBuilder;

use super::tool_error::ToolError;
use crate::{
    adapter::client::{ProjectTarget, resolve_project},
    domain::{port::ProjectStore, project::Project},
};

/// Where a server resolves projects from: the folder it was started in, and
/// the folder a call without `project` starts from (`--project`, or that
/// same folder).
#[derive(Debug, Clone, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Scope {
    /// The server's working directory, which a relative `project` folder is
    /// read against.
    cwd: PathBuf,
    /// The folder whose deepest containing project a call without `project`
    /// acts on.
    base: PathBuf,
}

impl Scope {
    /// The registered project a call acts on: the one `project` names (a
    /// project's name, or a folder inside it), else the deepest one
    /// containing the base folder.
    ///
    /// # Errors
    /// Returns [`ToolError::UnknownProject`] for a name no project has,
    /// [`ToolError::NoProjectHere`] when no project contains the folder, or
    /// the store's failure.
    pub fn project(
        &self,
        project: Option<&str>,
        projects: &dyn ProjectStore,
    ) -> Result<Project, ToolError> {
        Ok(self.target(project, projects)?.within(projects)?)
    }

    /// The project `process_add` adds to: like [`Scope::project`], except a
    /// folder no project is rooted at is its own, unregistered target, which
    /// the add registers.
    ///
    /// # Errors
    /// Returns [`ToolError::UnknownProject`] for a name no project has, the
    /// refusal of a folder that does not exist, or the store's failure.
    pub fn target(
        &self,
        project: Option<&str>,
        projects: &dyn ProjectStore,
    ) -> Result<ProjectTarget, ToolError> {
        let given = project.is_some_and(|value| !value.trim().is_empty());
        let from: &Path = if given { &self.cwd } else { &self.base };
        Ok(resolve_project(project, None, from, projects)?)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::store::{SqliteStore, test_support::spec},
        domain::{port::StackStore, value::ProjectRoot},
    };

    /// Registers the folder `dir` as a project by adding a process to it.
    fn register(store: &SqliteStore, dir: &Path) {
        store
            .add_process(
                &ProjectRoot::try_new(dir.to_path_buf()).unwrap(),
                &spec("web", "true"),
            )
            .unwrap();
    }

    /// A database registering `shop` and `shop/web` in a scratch tree that
    /// also holds `shop/web/src`, `shop/api` and `elsewhere`.
    fn tree() -> (TempDir, PathBuf, SqliteStore) {
        let scratch = TempDir::new().unwrap();
        let base = scratch.path().canonicalize().unwrap();
        for dir in ["shop/web/src", "shop/api", "elsewhere"] {
            fs::create_dir_all(base.join(dir)).unwrap();
        }
        let store = SqliteStore::open(&base.join("muster.db")).unwrap();
        for dir in ["shop", "shop/web"] {
            register(&store, &base.join(dir));
        }
        (scratch, base, store)
    }

    /// The scope of a server started in `cwd`.
    fn scope(cwd: &Path) -> Scope {
        Scope::builder()
            .cwd(cwd.to_path_buf())
            .base(cwd.to_path_buf())
            .build()
    }

    /// The name of `project`.
    fn name_of(project: &Project) -> String {
        project.name().to_string()
    }

    /// Without `project`, the deepest project containing the base folder
    /// wins; outside every project the call names the folder and
    /// process_add.
    #[test]
    fn the_deepest_containing_project_wins() {
        let (_scratch, base, store) = tree();

        let inner = scope(&base.join("shop/web/src"))
            .project(None, &store)
            .unwrap();
        assert_eq!(name_of(&inner), "web");
        let outer = scope(&base.join("shop/api")).project(None, &store).unwrap();
        assert_eq!(name_of(&outer), "shop");
        let outside = scope(&base.join("elsewhere"))
            .project(None, &store)
            .unwrap_err();
        assert!(
            matches!(&outside, ToolError::NoProjectHere { dir } if *dir == base.join("elsewhere"))
        );
        assert!(outside.to_string().contains("process_add"), "{outside}");
    }

    /// `project` as a name finds that project; an unknown name lists the
    /// registered ones; a folder is read against the server's directory.
    #[test]
    fn a_name_or_a_folder_picks_the_project() {
        let (_scratch, base, store) = tree();
        let scope = scope(&base.join("elsewhere"));

        assert_eq!(name_of(&scope.project(Some("web"), &store).unwrap()), "web");
        assert_eq!(
            name_of(&scope.project(Some("../shop/api"), &store).unwrap()),
            "shop"
        );
        let unknown = scope.project(Some("blog"), &store).unwrap_err();
        assert!(matches!(&unknown, ToolError::UnknownProject { valid, .. } if valid.len() == 2));
    }

    /// `--project` sets the base folder, while a folder given per call is
    /// read against the server's own directory.
    #[test]
    fn the_base_folder_serves_calls_without_project() {
        let (_scratch, base, store) = tree();
        let scope = Scope::builder()
            .cwd(base.join("elsewhere"))
            .base(base.join("shop/web"))
            .build();

        assert_eq!(name_of(&scope.project(None, &store).unwrap()), "web");
        assert!(matches!(
            scope.project(Some("."), &store),
            Err(ToolError::NoProjectHere { .. })
        ));
    }

    /// A folder registered after the server started is found on the next
    /// call.
    #[test]
    fn resolution_runs_per_call() {
        let (_scratch, base, store) = tree();
        let scope = scope(&base.join("elsewhere"));
        assert!(scope.project(None, &store).is_err());

        register(&store, &base.join("elsewhere"));

        assert_eq!(name_of(&scope.project(None, &store).unwrap()), "elsewhere");
    }

    /// `process_add` takes a folder no project is rooted at as its own
    /// target, and otherwise the containing project.
    #[test]
    fn process_add_targets_an_unregistered_folder() {
        let (_scratch, base, store) = tree();

        let outside = scope(&base.join("elsewhere")).target(None, &store).unwrap();
        assert_eq!(
            outside,
            ProjectTarget::Unregistered(ProjectRoot::try_new(base.join("elsewhere")).unwrap())
        );
        let nested = scope(&base.join("shop"))
            .target(Some("./api"), &store)
            .unwrap();
        assert_eq!(
            nested,
            ProjectTarget::Unregistered(ProjectRoot::try_new(base.join("shop/api")).unwrap())
        );
        let inside = scope(&base.join("shop/api")).target(None, &store).unwrap();
        assert!(
            matches!(inside, ProjectTarget::Registered(project) if project.name().to_string() == "shop")
        );
    }
}

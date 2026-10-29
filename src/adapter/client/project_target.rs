//! The one rule every command and tool call finds its project by.

use std::path::{Path, PathBuf};

use super::{refused_folder::RefusedFolder, resolve_error::ResolveError};
use crate::{
    adapter::path::{absolutize, expand_home, names_a_folder, project_root},
    domain::{
        port::ProjectStore,
        project::Project,
        value::{ProjectName, ProjectRoot},
    },
};

/// The project a command acts on: a registered one, or a folder that is not
/// a project yet, which only the commands that register one accept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectTarget {
    /// A registered project.
    Registered(Project),
    /// A folder no registered project is rooted at.
    Unregistered(ProjectRoot),
}

impl ProjectTarget {
    /// The folder the command acts on.
    pub fn root(&self) -> &ProjectRoot {
        match self {
            Self::Registered(project) => project.root(),
            Self::Unregistered(root) => root,
        }
    }

    /// The project's name, or the name registering the folder would give it
    /// were no other project to hold it.
    pub fn name(&self) -> Option<ProjectName> {
        match self {
            Self::Registered(project) => Some(project.name().clone()),
            Self::Unregistered(root) => ProjectName::candidates(root).next(),
        }
    }

    /// The registered project, for a command that acts on an existing one.
    ///
    /// # Errors
    /// Returns [`ResolveError::NotAProject`] for a folder that is not one.
    pub fn registered(self) -> Result<Project, ResolveError> {
        match self {
            Self::Registered(project) => Ok(project),
            Self::Unregistered(root) => Err(ResolveError::NotAProject {
                dir: root.into_inner(),
            }),
        }
    }

    /// This target, unless it is a folder that may not become a project: the
    /// filesystem root, `home` or a folder above it, or one holding a
    /// registered project.
    ///
    /// # Errors
    /// Returns [`ResolveError::Unregistrable`] naming why the folder is
    /// refused, or the store's failure.
    pub fn registrable(
        self,
        home: Option<&Path>,
        projects: &dyn ProjectStore,
    ) -> Result<Self, ResolveError> {
        let Self::Unregistered(root) = &self else {
            return Ok(self);
        };
        let dir: &Path = root.as_ref();
        let why = if dir.parent().is_none() {
            Some(RefusedFolder::FilesystemRoot)
        } else if home.is_some_and(|home| home == dir) {
            Some(RefusedFolder::Home)
        } else if home.is_some_and(|home| home.starts_with(dir)) {
            Some(RefusedFolder::AboveHome)
        } else {
            projects
                .projects()?
                .into_iter()
                .find(|project| project.root().as_ref().starts_with(dir))
                .map(|project| RefusedFolder::HoldsProject(project.name().clone()))
        };
        match why {
            Some(why) => Err(ResolveError::Unregistrable {
                dir: dir.to_path_buf(),
                why,
            }),
            None => Ok(self),
        }
    }

    /// The registered project, or for a folder no project is rooted at, the
    /// deepest one containing it: what an agent naming a folder inside a
    /// project means.
    ///
    /// # Errors
    /// Returns [`ResolveError::NotAProject`] when no project contains the
    /// folder, or the store's failure.
    pub fn within(self, projects: &dyn ProjectStore) -> Result<Project, ResolveError> {
        match self {
            Self::Registered(project) => Ok(project),
            Self::Unregistered(root) => {
                projects.project_containing(root.as_ref())?.ok_or_else(|| {
                    ResolveError::NotAProject {
                        dir: root.into_inner(),
                    }
                })
            },
        }
    }
}

/// Resolves the project a command acts on: `--project` (`explicit`), then
/// the root muster exports to each process (`env_project`), then the
/// deepest registered project containing `cwd`, else `cwd` unregistered.
/// A `--project` naming a folder (it contains `/` or starts with `.` or `~`)
/// is read against `cwd`, unless no such folder exists and a registered
/// project has that name, as `.dotfiles` may; anything else is a registered
/// project's name. `MUSTER_PROJECT` is a folder, or when no such folder
/// exists, a registered project's name; a bare word that is neither is an
/// unknown project. There is no fallback to some other registered project.
///
/// # Errors
/// Returns [`ResolveError::UnknownProject`] for an unregistered name, or the
/// failure of a folder that is neither a directory nor a registered root.
pub fn resolve_project(
    explicit: Option<&str>,
    env_project: Option<&Path>,
    cwd: &Path,
    projects: &dyn ProjectStore,
) -> Result<ProjectTarget, ResolveError> {
    let explicit = explicit.map(str::trim).filter(|value| !value.is_empty());
    if let Some(value) = explicit {
        if !names_a_folder(value) {
            return named(value, projects);
        }
        let path = cwd.join(expand_home(Path::new(value)));
        if !path.is_dir()
            && let Some(project) = registered_named(value, projects)?
        {
            return Ok(ProjectTarget::Registered(project));
        }
        return folder(&path, projects);
    }
    if let Some(value) = env_project {
        let path = cwd.join(value);
        if !path.is_dir()
            && let Some(name) = value.to_str()
        {
            if let Some(project) = registered_named(name, projects)? {
                return Ok(ProjectTarget::Registered(project));
            }
            if !names_a_folder(name) {
                return match folder(&path, projects) {
                    Err(ResolveError::Config(_)) => named(name, projects),
                    found => found,
                };
            }
        }
        return folder(&path, projects);
    }
    if let Some(project) = projects.project_containing(cwd)? {
        return Ok(ProjectTarget::Registered(project));
    }
    Ok(ProjectTarget::Unregistered(project_root(cwd)?))
}

/// The root a process muster started carries in its environment, when set.
pub fn env_project(value: Option<PathBuf>) -> Option<PathBuf> {
    value.filter(|path| !path.as_os_str().is_empty())
}

/// The registered project called `value`.
///
/// # Errors
/// Returns [`ResolveError::UnknownProject`] listing the registered names
/// when none is.
fn named(value: &str, projects: &dyn ProjectStore) -> Result<ProjectTarget, ResolveError> {
    match registered_named(value, projects)? {
        Some(project) => Ok(ProjectTarget::Registered(project)),
        None => Err(ResolveError::UnknownProject {
            name: value.to_string(),
            valid: projects
                .projects()?
                .iter()
                .map(|project| project.name().clone())
                .collect(),
        }),
    }
}

/// The registered project called `value`, if any.
///
/// # Errors
/// Returns the store's failure.
fn registered_named(
    value: &str,
    projects: &dyn ProjectStore,
) -> Result<Option<Project>, ResolveError> {
    match ProjectName::try_new(value) {
        Ok(name) => Ok(projects.project_named(&name)?),
        Err(_) => Ok(None),
    }
}

/// The project rooted at the folder `path`. A folder that no longer exists
/// on disk is still found when a project is registered at it, so its
/// processes can be listed and stopped.
///
/// # Errors
/// Returns the failure of a path that is not a directory and not a
/// registered root.
fn folder(path: &Path, projects: &dyn ProjectStore) -> Result<ProjectTarget, ResolveError> {
    match project_root(path) {
        Ok(root) => Ok(match projects.project_at(&root)? {
            Some(project) => ProjectTarget::Registered(project),
            None => ProjectTarget::Unregistered(root),
        }),
        Err(error) => {
            let gone = ProjectRoot::try_new(absolutize(path))
                .ok()
                .map(|root| projects.project_at(&root))
                .transpose()?
                .flatten();
            gone.map(ProjectTarget::Registered)
                .ok_or(ResolveError::Config(error))
        },
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::adapter::client::fixed_projects::FixedProjects;

    /// A project called `name` rooted at `root`.
    fn project(name: &str, root: &Path) -> Project {
        Project::builder()
            .root(ProjectRoot::try_new(root.to_path_buf()).unwrap())
            .name(ProjectName::try_new(name).unwrap())
            .build()
    }

    /// A scratch tree: `shop`, `shop/web` and `elsewhere`, canonicalized.
    fn tree() -> (TempDir, PathBuf) {
        let scratch = TempDir::new().unwrap();
        let base = scratch.path().canonicalize().unwrap();
        for dir in ["shop/web/src", "elsewhere"] {
            fs::create_dir_all(base.join(dir)).unwrap();
        }
        (scratch, base)
    }

    /// Inside nested projects the deepest one wins; outside every project
    /// the folder is unregistered, never some other registered project.
    #[test]
    fn the_deepest_containing_project_wins_and_nothing_else_falls_back() {
        let (_scratch, base) = tree();
        let shop = project("shop", &base.join("shop"));
        let web = project("web", &base.join("shop/web"));
        let projects = FixedProjects::new(vec![shop.clone(), web.clone()]);

        let resolved = resolve_project(None, None, &base.join("shop/web/src"), &projects).unwrap();
        assert_eq!(resolved, ProjectTarget::Registered(web));
        let resolved = resolve_project(None, None, &base.join("shop"), &projects).unwrap();
        assert_eq!(resolved, ProjectTarget::Registered(shop));
        let outside = resolve_project(None, None, &base.join("elsewhere"), &projects).unwrap();
        assert!(matches!(outside, ProjectTarget::Unregistered(_)));
        assert!(matches!(
            outside.registered(),
            Err(ResolveError::NotAProject { dir }) if dir == base.join("elsewhere")
        ));
    }

    /// The filesystem root, the home folder, a folder above it and a folder
    /// holding a project are refused; a folder inside home or inside a
    /// project, and a registered project, are not.
    #[test]
    fn only_a_folder_that_claims_nothing_else_may_be_registered() {
        let (_scratch, base) = tree();
        let home = base.join("elsewhere");
        let web = project("web", &base.join("shop/web"));
        let projects = FixedProjects::new(vec![web.clone()]);
        let unregistered = |dir: &Path| {
            ProjectTarget::Unregistered(ProjectRoot::try_new(dir.to_path_buf()).unwrap())
        };
        let refusal = |dir: &Path| match unregistered(dir).registrable(Some(&home), &projects) {
            Err(ResolveError::Unregistrable { why, .. }) => why,
            other => panic!("{} was not refused: {other:?}", dir.display()),
        };

        assert_eq!(refusal(Path::new("/")), RefusedFolder::FilesystemRoot);
        assert_eq!(refusal(&home), RefusedFolder::Home);
        assert_eq!(refusal(&base), RefusedFolder::AboveHome);
        assert_eq!(
            refusal(&base.join("shop")),
            RefusedFolder::HoldsProject(web.name().clone())
        );
        fs::create_dir_all(home.join("app")).unwrap();
        assert!(
            unregistered(&home.join("app"))
                .registrable(Some(&home), &projects)
                .is_ok()
        );
        assert!(
            unregistered(&base.join("shop/web/src"))
                .registrable(Some(&home), &projects)
                .is_ok()
        );
        assert!(
            ProjectTarget::Registered(web)
                .registrable(Some(&home), &projects)
                .is_ok()
        );
    }

    /// A folder named inside a project, which is no project's root, is read
    /// as the deepest project containing it; outside every project it is
    /// none.
    #[test]
    fn a_folder_inside_a_project_is_within_it() {
        let (_scratch, base) = tree();
        let shop = project("shop", &base.join("shop"));
        let projects = FixedProjects::new(vec![shop.clone()]);

        let nested = resolve_project(Some("./web/src"), None, &base.join("shop"), &projects)
            .unwrap()
            .within(&projects)
            .unwrap();
        assert_eq!(nested, shop);
        assert!(matches!(
            resolve_project(Some("."), None, &base.join("elsewhere"), &projects)
                .unwrap()
                .within(&projects),
            Err(ResolveError::NotAProject { .. })
        ));
    }

    /// `--project` beats the environment, which beats the folder; a name
    /// is looked up, a folder is read against the current directory.
    #[test]
    fn explicit_then_environment_then_folder() {
        let (_scratch, base) = tree();
        let shop = project("shop", &base.join("shop"));
        let web = project("web", &base.join("shop/web"));
        let projects = FixedProjects::new(vec![shop.clone(), web.clone()]);
        let cwd = base.join("shop/web");
        let env = base.join("shop");

        assert_eq!(
            resolve_project(Some("shop"), None, &cwd, &projects).unwrap(),
            ProjectTarget::Registered(shop.clone())
        );
        assert_eq!(
            resolve_project(None, Some(&env), &cwd, &projects).unwrap(),
            ProjectTarget::Registered(shop.clone())
        );
        assert_eq!(
            resolve_project(Some("."), Some(&env), &cwd, &projects).unwrap(),
            ProjectTarget::Registered(web),
            "`--project .` names the folder it is typed in"
        );
        assert_eq!(
            resolve_project(Some(".."), None, &cwd, &projects).unwrap(),
            ProjectTarget::Registered(shop)
        );
    }

    /// `MUSTER_PROJECT` holding a registered project's name, which is no
    /// folder, resolves to that project; a folder of that name wins, and a
    /// name that is neither is reported as an unknown project.
    #[test]
    fn the_environment_accepts_a_project_name() {
        let (_scratch, base) = tree();
        let shop = project("shop", &base.join("shop"));
        let elsewhere = project("elsewhere", &base.join("elsewhere"));
        let projects = FixedProjects::new(vec![shop.clone(), elsewhere.clone()]);

        assert_eq!(
            resolve_project(
                None,
                Some(Path::new("elsewhere")),
                &base.join("shop"),
                &projects
            )
            .unwrap(),
            ProjectTarget::Registered(elsewhere)
        );
        assert_eq!(
            resolve_project(None, Some(Path::new("shop")), &base, &projects).unwrap(),
            ProjectTarget::Registered(shop)
        );
        match resolve_project(None, Some(Path::new("nosuch")), &base, &projects) {
            Err(ResolveError::UnknownProject { name, .. }) => assert_eq!(name, "nosuch"),
            other => panic!("expected an unknown project, got {other:?}"),
        }
    }

    /// `--project .` inside a project makes the nested folder its own,
    /// unregistered target, for the commands that register one.
    #[test]
    fn a_nested_folder_is_its_own_target_when_named() {
        let (_scratch, base) = tree();
        let projects = FixedProjects::new(vec![project("shop", &base.join("shop"))]);
        let nested = base.join("shop/web");

        assert_eq!(
            resolve_project(Some("."), None, &nested, &projects).unwrap(),
            ProjectTarget::Unregistered(ProjectRoot::try_new(nested).unwrap())
        );
    }

    /// An unregistered name is refused, naming it.
    #[test]
    fn an_unknown_name_is_refused() {
        let (_scratch, base) = tree();
        let projects = FixedProjects::new(Vec::new());

        assert!(matches!(
            resolve_project(Some("blog"), None, &base, &projects),
            Err(ResolveError::UnknownProject { name, valid }) if name == "blog" && valid.is_empty()
        ));
    }

    /// A registered project whose folder was deleted is still found by its
    /// folder, so its processes can be listed and stopped; an unregistered
    /// missing folder is refused.
    #[test]
    fn a_registered_project_whose_folder_is_gone_is_still_found() {
        let (_scratch, base) = tree();
        let gone = base.join("gone");
        let projects = FixedProjects::new(vec![project("gone", &gone)]);

        assert_eq!(
            resolve_project(None, Some(&gone), &base, &projects).unwrap(),
            ProjectTarget::Registered(project("gone", &gone))
        );
        assert!(resolve_project(Some("./missing"), None, &base, &projects).is_err());
    }

    /// A registered name starting with `.` or `~` is found by name when no
    /// such folder exists; an existing folder of that name still wins.
    #[test]
    fn a_dotted_name_is_a_name_when_no_such_folder_exists() {
        let (_scratch, base) = tree();
        let dotfiles = project(".dotfiles", &base.join("shop"));
        let tilde = project("~notes", &base.join("shop/web"));
        let projects = FixedProjects::new(vec![dotfiles.clone(), tilde.clone()]);

        assert_eq!(
            resolve_project(Some(".dotfiles"), None, &base, &projects).unwrap(),
            ProjectTarget::Registered(dotfiles)
        );
        assert_eq!(
            resolve_project(Some("~notes"), None, &base, &projects).unwrap(),
            ProjectTarget::Registered(tilde)
        );
        fs::create_dir(base.join("elsewhere/.dotfiles")).unwrap();
        assert_eq!(
            resolve_project(Some(".dotfiles"), None, &base.join("elsewhere"), &projects).unwrap(),
            ProjectTarget::Unregistered(
                ProjectRoot::try_new(base.join("elsewhere/.dotfiles")).unwrap()
            )
        );
        assert!(resolve_project(Some(".missing"), None, &base, &projects).is_err());
    }

    /// Only a non-empty `MUSTER_PROJECT` names a project.
    #[test]
    fn an_empty_environment_value_is_ignored() {
        assert_eq!(env_project(Some(PathBuf::new())), None);
        assert_eq!(
            env_project(Some(PathBuf::from("/p"))),
            Some(PathBuf::from("/p"))
        );
    }
}

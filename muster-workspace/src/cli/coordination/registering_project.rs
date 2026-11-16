use std::cell::OnceCell;

use muster_domain::{config::Registration, port::LocalState, project::Project, value::ProjectRoot};

use crate::cli::{
    command_line::invocation::Invocation, coordination::project_source::ProjectSource,
    failure::error::CliError,
};

/// The project a shared write acts on, found, and outside every project
/// registered, through [`Invocation::sharing_project`] the first time the
/// write asks for it.
pub struct RegisteringProject<'a> {
    /// The run whose `--project` and folder name the project.
    invocation: &'a Invocation<'a>,
    /// The state the project is found in and registered into.
    store: &'a dyn LocalState,
    /// The project once asked for, and whether asking registered it.
    found: OnceCell<Registration>,
}

impl<'a> RegisteringProject<'a> {
    /// The project `invocation` names in `store`, not yet looked for.
    pub fn new(invocation: &'a Invocation<'a>, store: &'a dyn LocalState) -> Self {
        Self {
            invocation,
            store,
            found: OnceCell::new(),
        }
    }

    /// The project this write registered, when it registered one.
    pub fn registered(&self) -> Option<&Project> {
        self.found
            .get()
            .filter(|registration| *registration.newly_registered())
            .map(Registration::project)
    }
}

impl ProjectSource for RegisteringProject<'_> {
    /// Looks the project up, registering its folder, only on the first ask.
    fn root(&self) -> Result<ProjectRoot, CliError> {
        if let Some(registration) = self.found.get() {
            return Ok(registration.project().root().clone());
        }
        let registration = self.invocation.sharing_project(self.store)?;
        Ok(self
            .found
            .get_or_init(|| registration)
            .project()
            .root()
            .clone())
    }
}

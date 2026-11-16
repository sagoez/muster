use muster_domain::{
    config::{ConfigError, WritePlace, Written},
    coordination::Author,
    port::LocalState,
    value::ProjectRoot,
};

use crate::cli::{
    command_line::invocation::Invocation, coordination::project_source::ProjectSource,
    failure::error::CliError,
};

/// The project a coordination command of this run acts on, looked up only
/// when the command asks: as a read finds it, or as a shared write places
/// and registers it.
pub struct SharedProject<'a> {
    /// The run whose `--project` and folder name the project.
    invocation: &'a Invocation<'a>,
    /// The state the project is found in and registered into.
    store: &'a dyn LocalState,
}

impl<'a> SharedProject<'a> {
    /// The project `invocation` names in `store`, not yet looked for.
    pub fn new(invocation: &'a Invocation<'a>, store: &'a dyn LocalState) -> Self {
        Self { invocation, store }
    }
}

impl ProjectSource for SharedProject<'_> {
    fn root(&self) -> Result<ProjectRoot, CliError> {
        Ok(self.invocation.project(self.store)?.root().clone())
    }

    /// Places the write as [`Invocation::write_place`] finds it, and runs it
    /// through the stack editor, which tells a running service of a project
    /// it registered.
    fn share<T>(
        &self,
        by: &Author,
        write: impl FnOnce(&WritePlace) -> Result<Written<T>, ConfigError>,
    ) -> Result<Written<T>, CliError> {
        let place = self.invocation.write_place(self.store, by)?;
        Ok(self.invocation.editor(self.store).share(|| write(&place))?)
    }
}

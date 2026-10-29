use std::sync::Arc;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::{
    application::StackEditor,
    domain::{
        config::ConfigError,
        coordination::Author,
        port::{CoordinationStore, ProjectStore, SettingsStore, StackStore, StateChangeNotifier},
        project::Project,
        value::ProjectRoot,
    },
};

/// What the MCP server works with: the project it serves, who it writes as,
/// and the stores and notifier its tools drive. Cheap to clone, so a tool can
/// carry it onto a blocking thread.
#[derive(Clone, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct McpDeps {
    /// The project's folder, registered or not; the first `process_add`
    /// registers it.
    root: ProjectRoot,
    /// Who coordination writes are attributed to.
    author: Author,
    /// The registered projects.
    projects: Arc<dyn ProjectStore + Send + Sync>,
    /// The shared notes, todos and key-values.
    coordination: Arc<dyn CoordinationStore + Send + Sync>,
    /// Each project's stack definition.
    stacks: Arc<dyn StackStore + Send + Sync>,
    /// The machine's settings.
    settings: Arc<dyn SettingsStore + Send + Sync>,
    /// Told after each committed edit, so a running service picks it up.
    notifier: Arc<dyn StateChangeNotifier + Send + Sync>,
}

impl McpDeps {
    /// The stack editor over these stores.
    pub fn editor(&self) -> StackEditor<'_> {
        StackEditor::builder()
            .stacks(self.stacks.as_ref())
            .settings(self.settings.as_ref())
            .notifier(self.notifier.as_ref())
            .build()
    }

    /// The project at the served root.
    ///
    /// # Errors
    /// Returns [`ConfigError::UnknownProject`] while the folder is not yet a
    /// project, or the store's failure.
    pub fn project(&self) -> Result<Project, ConfigError> {
        self.projects
            .project_at(&self.root)?
            .ok_or_else(|| ConfigError::UnknownProject {
                root: self.root.as_ref().clone(),
            })
    }
}

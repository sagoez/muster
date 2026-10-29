use std::sync::Arc;

use getset::Getters;
use typed_builder::TypedBuilder;

use super::{scope::Scope, scope_target::ScopeTarget, tool_error::ToolError};
use crate::{
    adapter::client::ServiceAccess,
    application::StackEditor,
    domain::{
        port::{CoordinationStore, ProjectStore, SettingsStore, StackStore, StateChangeNotifier},
        project::Project,
    },
};

/// What the MCP server works with: where it resolves projects from, the
/// identity it was given, the service it drives, and the stores and notifier
/// its tools write through. Cheap to clone, so a tool can carry it onto a
/// blocking thread.
#[derive(Clone, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct McpDeps {
    /// Where each call resolves its project from.
    scope: Scope,
    /// The `--as` name, which beats every other source of the agent's id.
    agent: Option<String>,
    /// How the server reaches the muster service.
    service: Arc<dyn ServiceAccess>,
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

    /// The registered project a call acts on, from its `project` argument.
    ///
    /// # Errors
    /// Returns the caller's mistake for a project that does not resolve, or
    /// the store's failure.
    pub fn project(&self, project: Option<&str>) -> Result<Project, ToolError> {
        self.scope.project(project, self.projects.as_ref())
    }

    /// The project `process_add` adds to, registered or not.
    ///
    /// # Errors
    /// Returns the caller's mistake for a project that does not resolve, or
    /// the store's failure.
    pub fn target(&self, project: Option<&str>) -> Result<ScopeTarget, ToolError> {
        self.scope.target(project, self.projects.as_ref())
    }
}

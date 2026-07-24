use std::{collections::HashSet, path::Path};

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::{
    agent_session::{AgentSession, AgentSessionId},
    value::CommandLine,
};

/// One durable session the active workspace should represent at startup.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct SessionRestore {
    /// Durable record that remains the source of lifecycle truth.
    session: AgentSession,
    /// Resume command when the provider identity is usable.
    #[builder(default)]
    command: Option<CommandLine>,
}

/// Selects durable sessions that belong in one workspace without performing I/O.
pub struct SessionRestorer;

impl SessionRestorer {
    /// Returns open or pending sessions for `project` not already represented by a pane.
    pub fn for_project(
        sessions: impl IntoIterator<Item = AgentSession>,
        project: &Path,
        existing: &HashSet<AgentSessionId>,
        owns_project: impl Fn(&Path, &Path) -> bool,
        owner_is_live: impl Fn(&AgentSession) -> bool,
    ) -> Vec<SessionRestore> {
        sessions
            .into_iter()
            .filter(|session| {
                matches!(
                    session.state(),
                    crate::domain::agent_session::AgentSessionState::Pending
                        | crate::domain::agent_session::AgentSessionState::Open
                ) && owns_project(session.project(), project)
                    && !existing.contains(session.id())
                    && !owner_is_live(session)
            })
            .map(|session| {
                let command = session.restore_command();
                SessionRestore::builder()
                    .session(session)
                    .command(command)
                    .build()
            })
            .collect()
    }
}

//! The headings the main help lists its commands under.

use strum::{EnumIter, EnumMessage};

use crate::cli::command_line::command::CommandName;

/// A heading of the main help, with its commands in the order it lists
/// them. Every visible command belongs to one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, EnumMessage)]
pub(crate) enum CommandGroup {
    /// Running and watching processes.
    #[strum(message = "Processes")]
    Processes,
    /// Editing what a project runs.
    #[strum(message = "Stack")]
    Stack,
    /// What agents use and share.
    #[strum(message = "Agents and coordination")]
    AgentsAndCoordination,
    /// Setting muster up.
    #[strum(message = "Setup")]
    Setup,
}

impl CommandGroup {
    /// The heading, as the main help prints it.
    pub(crate) fn heading(self) -> &'static str {
        self.get_message().unwrap_or_default()
    }

    /// The group's commands, in the order the main help lists them, which
    /// is not the order `Command` declares them in.
    pub(crate) fn commands(self) -> &'static [CommandName] {
        match self {
            Self::Processes => &[
                CommandName::Ps,
                CommandName::Up,
                CommandName::Down,
                CommandName::Start,
                CommandName::Stop,
                CommandName::Restart,
                CommandName::Pause,
                CommandName::Resume,
                CommandName::Kill,
                CommandName::Wait,
                CommandName::Logs,
                CommandName::Attach,
                CommandName::Open,
                CommandName::Run,
                CommandName::View,
            ],
            Self::Stack => &[
                CommandName::Add,
                CommandName::Set,
                CommandName::Rm,
                CommandName::Import,
                CommandName::Export,
                CommandName::Init,
                CommandName::Projects,
            ],
            Self::AgentsAndCoordination => &[
                CommandName::Mcp,
                CommandName::Note,
                CommandName::Todo,
                CommandName::Kv,
                CommandName::Lock,
                CommandName::Coordination,
                CommandName::Notices,
            ],
            Self::Setup => &[
                CommandName::Config,
                CommandName::ShellInit,
                CommandName::Completions,
                CommandName::Doctor,
                CommandName::Service,
            ],
        }
    }
}

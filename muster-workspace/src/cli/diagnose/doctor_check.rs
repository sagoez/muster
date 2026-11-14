use serde::Serialize;
use strum::{EnumIter, EnumMessage};

/// What one `muster doctor` probe checks, named in kebab case under
/// `--json`. Its message is the label its row shows a person.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, EnumIter, EnumMessage)]
#[serde(rename_all = "kebab-case")]
pub enum DoctorCheck {
    /// The database opens and its settings read.
    #[strum(message = "database")]
    Database,
    /// The registered projects and whether their folders exist.
    #[strum(message = "projects")]
    Projects,
    /// Whether every stored process reads.
    #[strum(message = "processes")]
    Processes,
    /// The project the current folder belongs to.
    #[strum(message = "this folder")]
    ThisFolder,
    /// Files an older muster left behind.
    #[strum(message = "old files")]
    OldFiles,
    /// Whether the directories the service is reached through are private.
    #[strum(message = "socket")]
    Socket,
    /// Whether the shell registers muster's completions.
    #[strum(message = "completions")]
    Completions,
    /// Whether the shell doctor runs in loads muster's prompt hook.
    #[strum(message = "shell hook")]
    ShellHook,
    /// Whether doctor runs in a terminal workspace `muster open` can split,
    /// and whether the detach key is one such a workspace often takes.
    #[strum(message = "workspace")]
    Workspace,
}

#[cfg(test)]
mod tests {
    use strum::{EnumMessage, IntoEnumIterator};

    use super::DoctorCheck;

    /// Every check has a label for its row.
    #[test]
    fn every_check_has_a_label() {
        for check in DoctorCheck::iter() {
            assert!(check.get_message().is_some(), "{check:?}");
        }
    }
}

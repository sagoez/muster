use std::path::Path;

use super::{
    agent_config_error::AgentConfigError, edit_outcome::EditOutcome, entry_change::EntryChange,
    entry_match::EntryMatch, server_name::McpServerName,
};

/// What is done to the table of servers, decided the same way for every
/// format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EntryAction {
    /// muster's entry goes in under the name.
    Insert,
    /// muster's older entry is pointed at this binary, its other keys kept.
    Update,
    /// The entry under the name is replaced by muster's.
    Replace,
    /// The entry under the name is taken out.
    Remove,
    /// Nothing is written, for this reason.
    Keep(EditOutcome),
}

impl EntryAction {
    /// The action making `change` where `existing` is what is under the
    /// name, with whether it is turned off, in the file at `path`.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::Taken`] or [`AgentConfigError::NotOurs`]
    /// for another server's entry under `name` without `force`.
    pub(super) fn decide(
        existing: Option<(EntryMatch, bool)>,
        change: EntryChange,
        force: bool,
        path: &Path,
        name: &McpServerName,
    ) -> Result<Self, AgentConfigError> {
        Ok(match (change, existing) {
            (EntryChange::Add, None) => Self::Insert,
            (EntryChange::Add, Some((EntryMatch::Foreign, _))) if !force => {
                return Err(AgentConfigError::Taken {
                    path: path.to_path_buf(),
                    name: name.clone(),
                });
            },
            (EntryChange::Add, Some((EntryMatch::Foreign, _) | (_, true))) if force => {
                Self::Replace
            },
            (EntryChange::Add, Some((_, true))) => Self::Keep(EditOutcome::Disabled),
            (EntryChange::Add, Some((EntryMatch::Older, _))) => Self::Update,
            (EntryChange::Add, Some(_)) => Self::Keep(EditOutcome::AlreadyAdded),
            (EntryChange::Remove, None) => Self::Keep(EditOutcome::Absent),
            (EntryChange::Remove, Some((EntryMatch::Foreign, _))) if !force => {
                return Err(AgentConfigError::NotOurs {
                    path: path.to_path_buf(),
                    name: name.clone(),
                });
            },
            (EntryChange::Remove, Some(_)) => Self::Remove,
        })
    }

    /// What the action does to the config.
    pub(super) fn outcome(self) -> EditOutcome {
        match self {
            Self::Insert => EditOutcome::Added,
            Self::Update => EditOutcome::Updated,
            Self::Replace => EditOutcome::Replaced,
            Self::Remove => EditOutcome::Removed,
            Self::Keep(outcome) => outcome,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::agent_config::testing::muster;

    /// The action for `existing` and `change`, or the refusal.
    fn decided(
        existing: Option<(EntryMatch, bool)>,
        change: EntryChange,
        force: bool,
    ) -> Result<EntryAction, AgentConfigError> {
        EntryAction::decide(existing, change, force, Path::new("/cfg"), &muster())
    }

    /// muster's own entry at another path is updated without force, a
    /// turned-off one is reported and turned on only with force, and
    /// another server's is replaced or removed only with force.
    #[test]
    fn force_is_needed_only_for_another_server_or_a_turned_off_one() {
        use EntryMatch::{Foreign, Older, Same};
        for (existing, add, add_forced, remove) in [
            (
                None,
                EntryAction::Insert,
                EntryAction::Insert,
                EntryAction::Keep(EditOutcome::Absent),
            ),
            (
                Some((Same, false)),
                EntryAction::Keep(EditOutcome::AlreadyAdded),
                EntryAction::Keep(EditOutcome::AlreadyAdded),
                EntryAction::Remove,
            ),
            (
                Some((Older, false)),
                EntryAction::Update,
                EntryAction::Update,
                EntryAction::Remove,
            ),
            (
                Some((Same, true)),
                EntryAction::Keep(EditOutcome::Disabled),
                EntryAction::Replace,
                EntryAction::Remove,
            ),
            (
                Some((Older, true)),
                EntryAction::Keep(EditOutcome::Disabled),
                EntryAction::Replace,
                EntryAction::Remove,
            ),
        ] {
            assert_eq!(
                decided(existing, EntryChange::Add, false).unwrap(),
                add,
                "{existing:?}"
            );
            assert_eq!(
                decided(existing, EntryChange::Add, true).unwrap(),
                add_forced,
                "{existing:?}"
            );
            assert_eq!(
                decided(existing, EntryChange::Remove, false).unwrap(),
                remove,
                "{existing:?}"
            );
        }
        for disabled in [false, true] {
            let foreign = Some((Foreign, disabled));
            assert!(matches!(
                decided(foreign, EntryChange::Add, false),
                Err(AgentConfigError::Taken { .. })
            ));
            assert!(matches!(
                decided(foreign, EntryChange::Remove, false),
                Err(AgentConfigError::NotOurs { .. })
            ));
            assert_eq!(
                decided(foreign, EntryChange::Add, true).unwrap(),
                EntryAction::Replace
            );
            assert_eq!(
                decided(foreign, EntryChange::Remove, true).unwrap(),
                EntryAction::Remove
            );
        }
    }
}

use strum::{EnumProperty, EnumString};

/// The property saying a command publishes the checkout.
const PUBLISHES: &str = "publishes";

/// A git subcommand that changes a working tree or publishes it, so two
/// agents in one checkout would step on each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumString, EnumProperty)]
#[strum(serialize_all = "kebab-case")]
pub enum GitCommand {
    /// `git switch`.
    Switch,
    /// `git checkout`.
    Checkout,
    /// `git commit`.
    Commit,
    /// `git rebase`.
    Rebase,
    /// `git reset`.
    Reset,
    /// `git stash`.
    Stash,
    /// `git merge`.
    Merge,
    /// `git pull`.
    Pull,
    /// `git push`.
    #[strum(props(publishes = true))]
    Push,
    /// `git cherry-pick`.
    CherryPick,
    /// `git restore`.
    Restore,
}

impl GitCommand {
    /// Whether it publishes the checkout, after which its todo should carry
    /// the branch.
    pub fn publishes(self) -> bool {
        self.get_bool(PUBLISHES).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    /// Subcommands parse as git spells them, a read-only one does not, and
    /// only a push publishes.
    #[test]
    fn subcommands_parse_as_git_spells_them() {
        assert_eq!(
            GitCommand::from_str("cherry-pick"),
            Ok(GitCommand::CherryPick)
        );
        assert!(GitCommand::from_str("status").is_err());
        assert!(GitCommand::Push.publishes());
        assert!(!GitCommand::Commit.publishes());
    }
}

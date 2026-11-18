use strum::EnumString;

/// Starts an option word.
const OPTION_LEAD: char = '-';
/// `git stash`'s subcommands that only read.
const READING_STASH: [&str; 2] = ["list", "show"];
/// `git push`'s flags that only try the push.
const PUSH_DRY_RUN: [&str; 2] = ["--dry-run", "-n"];
/// The flag that only tries a commit; its `-n` skips hooks instead.
const DRY_RUN: &str = "--dry-run";
/// The one `git worktree` subcommand that destroys a tree.
const WORKTREE_REMOVE: &str = "remove";

/// A git subcommand that changes a working tree or publishes it, so two
/// agents in one checkout would step on each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumString)]
#[strum(serialize_all = "kebab-case")]
pub enum GitCommand {
    /// `git switch`.
    Switch,
    /// `git checkout`.
    Checkout,
    /// `git commit`, but for a dry run.
    Commit,
    /// `git rebase`.
    Rebase,
    /// `git reset`.
    Reset,
    /// `git stash`, but for `list` and `show`.
    Stash,
    /// `git merge`.
    Merge,
    /// `git pull`.
    Pull,
    /// `git push`, but for a dry run.
    Push,
    /// `git cherry-pick`.
    CherryPick,
    /// `git restore`.
    Restore,
    /// `git revert`.
    Revert,
    /// `git am`.
    Am,
    /// `git clean`.
    Clean,
    /// `git worktree remove`.
    Worktree,
    /// `git rm`.
    Rm,
    /// `git mv`.
    Mv,
    /// `git apply`.
    Apply,
}

impl GitCommand {
    /// Whether it changes or publishes the checkout when run with `args`,
    /// the words after it: the reading forms of stash, a dry-run push or
    /// commit and every worktree subcommand but `remove` do not.
    pub fn changes_tree(self, args: &[String]) -> bool {
        let first = args.iter().find(|word| !word.starts_with(OPTION_LEAD));
        match self {
            Self::Stash => !first.is_some_and(|word| READING_STASH.contains(&word.as_str())),
            Self::Push => !args
                .iter()
                .any(|word| PUSH_DRY_RUN.contains(&word.as_str())),
            Self::Commit => !args.iter().any(|word| word == DRY_RUN),
            Self::Worktree => first.is_some_and(|word| word == WORKTREE_REMOVE),
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    /// The words of `line`.
    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    /// Subcommands parse as git spells them, and a read-only one does not.
    #[test]
    fn subcommands_parse_as_git_spells_them() {
        assert_eq!(
            GitCommand::from_str("cherry-pick"),
            Ok(GitCommand::CherryPick)
        );
        assert_eq!(GitCommand::from_str("am"), Ok(GitCommand::Am));
        assert!(GitCommand::from_str("status").is_err());
    }

    /// The reading forms of a changing command leave the tree alone; a
    /// commit's `-n` still commits.
    #[test]
    fn reading_forms_change_nothing() {
        assert!(!GitCommand::Stash.changes_tree(&words("list")));
        assert!(!GitCommand::Stash.changes_tree(&words("show -p stash@{0}")));
        assert!(GitCommand::Stash.changes_tree(&words("")));
        assert!(GitCommand::Stash.changes_tree(&words("pop")));
        assert!(!GitCommand::Push.changes_tree(&words("--dry-run origin main")));
        assert!(GitCommand::Push.changes_tree(&words("origin main")));
        assert!(GitCommand::Worktree.changes_tree(&words("remove ../w")));
        assert!(!GitCommand::Worktree.changes_tree(&words("add ../w")));
        assert!(GitCommand::Clean.changes_tree(&words("-fd")));
        assert!(!GitCommand::Commit.changes_tree(&words("--dry-run -am x")));
        assert!(GitCommand::Commit.changes_tree(&words("-n -am x")));
    }
}

use strum::{EnumProperty, EnumString};

/// Starts an option word.
pub const OPTION_LEAD: char = '-';
/// Flags that only try or describe any guarded git or gh command.
const READ_ONLY_FLAGS: [&str; 3] = ["--dry-run", "-h", "--help"];
/// The property holding a command's own read-only flags, space separated.
const READING_FLAGS: &str = "reading_flags";
/// `git stash`'s subcommands that only read.
const READING_STASH: [&str; 2] = ["list", "show"];
/// Makes `git apply` apply again after its read-only flags.
const APPLY_ANYWAY: &str = "--apply";

/// A git subcommand that changes a working tree or publishes it, so two
/// agents in one checkout would step on each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumString, EnumProperty)]
#[strum(serialize_all = "kebab-case")]
pub enum GitCommand {
    /// `git switch`.
    Switch,
    /// `git checkout`.
    Checkout,
    /// `git commit`; its `-n` skips hooks and still commits.
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
    /// `git push`.
    #[strum(props(reading_flags = "-n"))]
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
    #[strum(props(reading_flags = "-n"))]
    Clean,
    /// `git rm`.
    #[strum(props(reading_flags = "-n"))]
    Rm,
    /// `git mv`.
    #[strum(props(reading_flags = "-n"))]
    Mv,
    /// `git apply`.
    #[strum(props(reading_flags = "--check --stat --numstat --summary"))]
    Apply,
}

impl GitCommand {
    /// Whether it changes or publishes the checkout when run with `args`,
    /// the words after it: a dry run, a help request, its own read-only
    /// flags, a one-letter one bundled with others as in `-nd`, and the
    /// reading forms of stash do not; `--apply` makes `apply` apply anyway.
    pub fn changes_tree(self, args: &[String]) -> bool {
        let own: Vec<&str> = self
            .get_str(READING_FLAGS)
            .unwrap_or_default()
            .split_whitespace()
            .collect();
        let reads = |word: &String| {
            own.contains(&word.as_str()) || own.iter().any(|flag| is_bundled_in(flag, word))
        };
        let first = args.iter().find(|word| !word.starts_with(OPTION_LEAD));
        let applies = self == Self::Apply && args.iter().any(|word| word == APPLY_ANYWAY);
        !is_read_only(args)
            && (applies || !args.iter().any(reads))
            && !(self == Self::Stash
                && first.is_some_and(|word| READING_STASH.contains(&word.as_str())))
    }
}

/// Whether the one-letter `flag`, such as `-n`, is among the letters of the
/// bundled short options `word`, such as `-nd`.
fn is_bundled_in(flag: &str, word: &str) -> bool {
    let mut letters = flag.chars().skip(1);
    match (letters.next(), letters.next()) {
        (Some(letter), None) => {
            word.starts_with(OPTION_LEAD)
                && !word[1..].starts_with(OPTION_LEAD)
                && word[1..]
                    .chars()
                    .all(|character| character.is_ascii_alphabetic())
                && word[1..].contains(letter)
        },
        _ => false,
    }
}

/// Whether `args` only try or describe a command: a dry run or a help
/// request.
pub fn is_read_only(args: &[String]) -> bool {
    args.iter()
        .any(|word| READ_ONLY_FLAGS.contains(&word.as_str()))
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
        assert!(GitCommand::from_str("worktree").is_err());
    }

    /// The reading forms of a changing command leave the tree alone; a
    /// commit's `-n` still commits.
    #[test]
    fn reading_forms_change_nothing() {
        for (command, args) in [
            (GitCommand::Stash, "list"),
            (GitCommand::Stash, "show -p stash@{0}"),
            (GitCommand::Push, "--dry-run origin main"),
            (GitCommand::Push, "-n origin main"),
            (GitCommand::Commit, "--dry-run -am x"),
            (GitCommand::Commit, "-h"),
            (GitCommand::Clean, "-n -d"),
            (GitCommand::Rm, "-n --cached x"),
            (GitCommand::Mv, "-n a b"),
            (GitCommand::Apply, "--check p.patch"),
            (GitCommand::Restore, "--help"),
            (GitCommand::Clean, "-nd"),
            (GitCommand::Clean, "-dnx"),
            (GitCommand::Rm, "-rn dir"),
            (GitCommand::Push, "-nu origin main"),
        ] {
            assert!(!command.changes_tree(&words(args)), "{command:?} {args}");
        }
        for (command, args) in [
            (GitCommand::Stash, ""),
            (GitCommand::Stash, "pop"),
            (GitCommand::Push, "origin main"),
            (GitCommand::Push, "--no-verify origin main"),
            (GitCommand::Commit, "-n -am x"),
            (GitCommand::Clean, "-fd"),
            (GitCommand::Apply, "p.patch"),
            (GitCommand::Apply, "--stat --apply p.patch"),
        ] {
            assert!(command.changes_tree(&words(args)), "{command:?} {args}");
        }
        let empty = vec!["-m".to_owned(), String::new()];
        assert!(GitCommand::Commit.changes_tree(&empty), "an empty argument");
    }

    /// The property name read in code is the one the variants carry, so the
    /// read-only flags are never silently lost.
    #[test]
    fn the_reading_flags_property_is_read() {
        assert!(GitCommand::Push.get_str(READING_FLAGS).is_some());
    }
}

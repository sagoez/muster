use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::cli::mcp::guard::{git_command::GitCommand, pr_command::PrCommand};

/// Characters that end one simple command and start the next, outside quotes.
const SEPARATORS: [char; 6] = [';', '&', '|', '\n', '(', ')'];
/// Opens and closes a single-quoted word.
const SINGLE_QUOTE: char = '\'';
/// Opens and closes a double-quoted word.
const DOUBLE_QUOTE: char = '"';
/// Escapes the next character outside single quotes.
const BACKSLASH: char = '\\';
/// Joins a variable to its value in a leading assignment.
const ASSIGN: char = '=';
/// Starts an option word.
const OPTION_LEAD: char = '-';
/// Starts a long option word, which is never a shell's `-c`.
const LONG_OPTION_LEAD: &str = "--";
/// The one punctuation a variable name may hold.
const NAME_PUNCTUATION: char = '_';
/// The letter of a shell's run-this-script flag, as in `-c` or `-lc`.
const SCRIPT_FLAG: char = 'c';
/// Commands that run the rest of their words as a command.
const WRAPPERS: [&str; 5] = ["command", "env", "exec", "nohup", "time"];
/// Shells whose `-c` script is read as commands of its own.
const SHELLS: [&str; 4] = ["sh", "bash", "zsh", "dash"];
/// Changes the folder later commands run in.
const CD: &str = "cd";
/// git itself.
const GIT: &str = "git";
/// git's flag naming the folder it runs in.
const GIT_DIR_FLAG: &str = "-C";
/// git's options before the subcommand that take the next word as a value.
const GIT_VALUE_OPTIONS: [&str; 6] = [
    GIT_DIR_FLAG,
    "-c",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--config-env",
];
/// GitHub's CLI.
const GH: &str = "gh";
/// gh's pull request command group.
const GH_PR: &str = "pr";
/// How many shell `-c` scripts deep a command is still read.
const MAX_SCRIPT_DEPTH: usize = 2;

/// A shell command line that runs a git or gh command changing or
/// publishing a checkout: the folder the first such command runs in, and
/// whether any of them publishes.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct ShellCall {
    /// The folder the first guarded command runs in.
    #[getset(get = "pub")]
    dir: PathBuf,
    /// Whether a `git push` or `gh pr create` is among them.
    #[getset(get_copy = "pub")]
    publishes: bool,
}

impl ShellCall {
    /// The guarded commands `command`, run in `cwd`, holds, read command by
    /// command, following `cd`, `git -C`, leading assignments, a few
    /// wrappers and shell `-c` scripts; none when it holds none. A best
    /// effort, not a shell parser.
    pub fn parse(command: &str, cwd: &Path) -> Option<Self> {
        let mut dir = cwd.to_path_buf();
        let mut found = Vec::new();
        scan(command, &mut dir, &mut found, 0);
        let (dir, _) = found.first()?.clone();
        Some(Self {
            dir,
            publishes: found.iter().any(|(_, publishes)| *publishes),
        })
    }
}

/// Pushes each guarded command of `script` onto `found`, with the folder it
/// runs in and whether it publishes, tracking `cd` in `dir`.
fn scan(script: &str, dir: &mut PathBuf, found: &mut Vec<(PathBuf, bool)>, depth: usize) {
    for segment in segments(script) {
        let Some(words) = shlex::split(&segment) else {
            continue;
        };
        let words = command_words(&words);
        let Some((program, args)) = words.split_first() else {
            continue;
        };
        match program_name(program) {
            CD => {
                if let Some(target) = args.first() {
                    *dir = dir.join(target);
                }
            },
            GIT => {
                if let Some(call) = git_call(args, dir) {
                    found.push(call);
                }
            },
            GH => {
                if let Some(publishes) = pr_call(args) {
                    found.push((dir.clone(), publishes));
                }
            },
            shell if SHELLS.contains(&shell) && depth < MAX_SCRIPT_DEPTH => {
                if let Some(inner) = shell_script(args) {
                    scan(inner, &mut dir.clone(), found, depth + 1);
                }
            },
            _ => {},
        }
    }
}

/// `script` cut into simple commands at every separator outside quotes.
fn segments(script: &str) -> Vec<String> {
    let mut segments = vec![String::new()];
    let (mut single, mut double, mut escaped) = (false, false, false);
    for character in script.chars() {
        if escaped {
            escaped = false;
        } else if character == BACKSLASH && !single {
            escaped = true;
        } else if character == SINGLE_QUOTE && !double {
            single = !single;
        } else if character == DOUBLE_QUOTE && !single {
            double = !double;
        } else if SEPARATORS.contains(&character) && !single && !double {
            segments.push(String::new());
            continue;
        }
        if let Some(current) = segments.last_mut() {
            current.push(character);
        }
    }
    segments
}

/// `words` past leading variable assignments and wrapper commands.
fn command_words(words: &[String]) -> &[String] {
    let mut rest = words;
    while let Some((first, tail)) = rest.split_first() {
        if is_assignment(first) || WRAPPERS.contains(&program_name(first)) {
            rest = tail;
        } else {
            break;
        }
    }
    rest
}

/// Whether `word` assigns a variable, as `NAME=value`.
fn is_assignment(word: &str) -> bool {
    word.split_once(ASSIGN).is_some_and(|(name, _)| {
        name.chars()
            .next()
            .is_some_and(|first| !first.is_ascii_digit())
            && name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == NAME_PUNCTUATION)
    })
}

/// The file name a program word runs, so `/usr/bin/git` reads as `git`.
fn program_name(word: &str) -> &str {
    Path::new(word)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(word)
}

/// The folder a `git` with `args` runs in and whether it publishes, when
/// its subcommand is guarded.
fn git_call(args: &[String], dir: &Path) -> Option<(PathBuf, bool)> {
    let mut dir = dir.to_path_buf();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if GIT_VALUE_OPTIONS.contains(&word.as_str()) {
            let value = words.next()?;
            if word == GIT_DIR_FLAG {
                dir = dir.join(value);
            }
        } else if !word.starts_with(OPTION_LEAD) {
            let command = GitCommand::from_str(word).ok()?;
            return Some((dir, command.publishes()));
        }
    }
    None
}

/// Whether a `gh` with `args` publishes, when it is a guarded `gh pr`.
fn pr_call(args: &[String]) -> Option<bool> {
    let mut words = args.iter().filter(|word| !word.starts_with(OPTION_LEAD));
    if words.next()? != GH_PR {
        return None;
    }
    PrCommand::from_str(words.next()?)
        .ok()
        .map(PrCommand::publishes)
}

/// The script a shell with `args` runs through its `-c` flag.
fn shell_script(args: &[String]) -> Option<&str> {
    let mut words = args.iter();
    words.by_ref().find(|word| {
        word.starts_with(OPTION_LEAD)
            && !word.starts_with(LONG_OPTION_LEAD)
            && word.contains(SCRIPT_FLAG)
    })?;
    words.next().map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The folder every test command runs in.
    const CWD: &str = "/work/shop";

    /// What `command` run in [`CWD`] is found to be.
    fn parsed(command: &str) -> Option<ShellCall> {
        ShellCall::parse(command, Path::new(CWD))
    }

    /// The call guarding `dir`, publishing or not.
    fn call(dir: &str, publishes: bool) -> Option<ShellCall> {
        Some(
            ShellCall::builder()
                .dir(PathBuf::from(dir))
                .publishes(publishes)
                .build(),
        )
    }

    /// Commands that change or publish a tree are found; reading ones are not.
    #[test]
    fn tree_changing_commands_are_found_and_reading_ones_are_not() {
        assert_eq!(parsed("git commit -m 'a; b'"), call(CWD, false));
        assert_eq!(parsed("git push origin HEAD"), call(CWD, true));
        assert_eq!(parsed("gh pr create --fill"), call(CWD, true));
        assert_eq!(parsed("gh pr merge 12 --squash"), call(CWD, false));
        for reading in [
            "git status",
            "git log --oneline -3",
            "gh pr view 12",
            "echo 'git push'",
            "cargo test",
            "",
        ] {
            assert_eq!(parsed(reading), None, "{reading}");
        }
    }

    /// The folder follows `git -C`, `cd` and absolute paths, and an option
    /// value is never read as the subcommand.
    #[test]
    fn the_folder_follows_cd_and_git_c() {
        assert_eq!(
            parsed("git -C ../api switch main"),
            call("/work/shop/../api", false)
        );
        assert_eq!(parsed("cd /work/api && git pull"), call("/work/api", false));
        assert_eq!(parsed("git -c core.pager=cat commit"), call(CWD, false));
        assert_eq!(parsed("git -c push status"), None);
    }

    /// Leading assignments, wrappers, a full path, a pipeline and a shell
    /// script still find the command, and any publishing one marks the call.
    #[test]
    fn wrapped_and_chained_commands_are_found() {
        assert_eq!(
            parsed("GIT_EDITOR=true git rebase --continue"),
            call(CWD, false)
        );
        assert_eq!(parsed("env FOO=1 /usr/bin/git stash"), call(CWD, false));
        assert_eq!(
            parsed("git status; git commit -am x && git push"),
            call(CWD, true)
        );
        assert_eq!(
            parsed("bash -lc 'cd /work/api && git push'"),
            call("/work/api", true)
        );
        assert_eq!(parsed("echo done | git cherry-pick abc"), call(CWD, false));
    }

    /// An unclosed quote leaves its command unread rather than misread.
    #[test]
    fn an_unclosed_quote_is_skipped() {
        assert_eq!(parsed("git commit -m 'oops"), None);
    }
}

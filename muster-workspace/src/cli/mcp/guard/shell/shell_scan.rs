use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::cli::mcp::guard::shell::{
    git_command::{GitCommand, OPTION_LEAD, is_read_only},
    pr_command::PrCommand,
};

/// git itself.
pub const GIT: &str = "git";
/// git's flag naming the folder it runs in.
pub const GIT_DIR_FLAG: &str = "-C";
/// Characters that end one simple command and start the next, outside quotes.
const SEPARATORS: [char; 6] = [';', '&', '|', '\n', '(', ')'];
/// Ends a line, and with it a comment.
const NEWLINE: char = '\n';
/// Opens and closes a single-quoted word.
const SINGLE_QUOTE: char = '\'';
/// Opens and closes a double-quoted word.
const DOUBLE_QUOTE: char = '"';
/// Escapes the next character outside single quotes.
const BACKSLASH: char = '\\';
/// Starts a comment at the start of a word.
const COMMENT: char = '#';
/// Opens a redirection; doubled, a heredoc or a here-string, after which
/// nothing more is read.
const REDIRECT_IN: char = '<';
/// Joins a variable to its value in a leading assignment.
const ASSIGN: char = '=';
/// The one punctuation a variable name may hold.
const NAME_PUNCTUATION: char = '_';
/// Shell keywords a command may follow.
const KEYWORDS: [&str; 9] = [
    "if", "then", "else", "elif", "while", "until", "do", "!", "{",
];
/// How a word names the home folder.
const HOME_SPELLINGS: [&str; 3] = ["~", "$HOME", "${HOME}"];
/// Separates a path's folders.
const PATH_SEPARATOR: char = '/';
/// Changes the folder later commands run in.
const CD: &str = "cd";
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
/// gh's options that take the next word as a value.
const GH_VALUE_OPTIONS: [&str; 2] = ["-R", "--repo"];
/// gh's repository option with its value in the same word.
const GH_REPO_ASSIGNED: &str = "--repo=";
/// gh's pull request command group.
const GH_PR: &str = "pr";

/// What the git and gh commands of `command`, run in `cwd`, change or
/// publish, each once, in order: the folder whose checkout one changes, or
/// none for a `gh pr` naming another repository with `-R`, which touches no
/// local checkout; empty when it runs no such command. It reads command by
/// command, following `cd` and `git -C`, reading `~` and `$HOME` as `home`,
/// past leading assignments and shell keywords, skipping comments and
/// stopping at the first heredoc. A best effort, not a shell parser.
pub fn guarded_dirs(command: &str, cwd: &Path, home: Option<&Path>) -> Vec<Option<PathBuf>> {
    let mut dir = cwd.to_path_buf();
    let mut found: Vec<Option<PathBuf>> = Vec::new();
    for segment in segments(command) {
        let Some(mut words) = shlex::split(&segment) else {
            continue;
        };
        words.retain(|word| !word.is_empty());
        let Some((program, args)) = command_words(&words).split_first() else {
            continue;
        };
        let guarded = match program_name(program) {
            CD => {
                match args.last() {
                    Some(target) if !target.starts_with(OPTION_LEAD) => {
                        dir = joined(&dir, target, home);
                    },
                    None => dir = home.map_or(dir, Path::to_path_buf),
                    Some(_) => {},
                }
                None
            },
            GIT => git_dir(args, &dir, home).map(Some),
            GH => guarded_pr(args).map(|local| local.then(|| dir.clone())),
            _ => None,
        };
        if let Some(guarded) = guarded
            && !found.contains(&guarded)
        {
            found.push(guarded);
        }
    }
    found
}

/// `script` cut into simple commands at every separator outside quotes,
/// leaving out comments, and ending at the first heredoc or here-string
/// outside quotes, whose body is not read.
fn segments(script: &str) -> Vec<String> {
    let chars: Vec<char> = script.chars().collect();
    let mut segments = vec![String::new()];
    let (mut single, mut double) = (false, false);
    let mut at = 0;
    while let Some(&character) = chars.get(at) {
        let quoted = single || double;
        at += 1;
        if character == BACKSLASH && !single {
            push(&mut segments, character);
            if let Some(&next) = chars.get(at) {
                push(&mut segments, next);
                at += 1;
            }
            continue;
        }
        if character == SINGLE_QUOTE && !double {
            single = !single;
        } else if character == DOUBLE_QUOTE && !single {
            double = !double;
        } else if !quoted && character == COMMENT && starts_word(&chars, at - 1) {
            while chars.get(at).is_some_and(|next| *next != NEWLINE) {
                at += 1;
            }
            continue;
        } else if !quoted && character == REDIRECT_IN && chars.get(at) == Some(&REDIRECT_IN) {
            break;
        } else if !quoted && SEPARATORS.contains(&character) {
            segments.push(String::new());
            continue;
        }
        push(&mut segments, character);
    }
    segments
}

/// Adds `character` to the last of `segments`.
fn push(segments: &mut [String], character: char) {
    if let Some(current) = segments.last_mut() {
        current.push(character);
    }
}

/// Whether the character at `at` of `chars` starts a word.
fn starts_word(chars: &[char], at: usize) -> bool {
    at == 0
        || chars
            .get(at - 1)
            .is_some_and(|before| before.is_whitespace() || SEPARATORS.contains(before))
}

/// `words` past leading variable assignments and shell keywords.
fn command_words(words: &[String]) -> &[String] {
    let skipped = words
        .iter()
        .take_while(|word| is_assignment(word) || KEYWORDS.contains(&word.as_str()))
        .count();
    &words[skipped..]
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

/// `word` as a folder against `dir`, `~` and `$HOME` read as `home`.
fn joined(dir: &Path, word: &str, home: Option<&Path>) -> PathBuf {
    let rest = HOME_SPELLINGS
        .iter()
        .find_map(|spelling| {
            word.strip_prefix(spelling)
                .filter(|rest| rest.is_empty() || rest.starts_with(PATH_SEPARATOR))
        })
        .map(|rest| rest.trim_start_matches(PATH_SEPARATOR));
    match (rest, home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => dir.join(word),
    }
}

/// The file name a program word runs, so `/usr/bin/git` reads as `git`.
fn program_name(word: &str) -> &str {
    Path::new(word)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(word)
}

/// The folder a `git` with `args`, run in `dir`, changes or publishes,
/// when its subcommand does.
fn git_dir(args: &[String], dir: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let mut dir = dir.to_path_buf();
    let mut words = args.iter().enumerate();
    while let Some((index, word)) = words.next() {
        if GIT_VALUE_OPTIONS.contains(&word.as_str()) {
            let (_, value) = words.next()?;
            if word == GIT_DIR_FLAG {
                dir = joined(&dir, value, home);
            }
        } else if !word.starts_with(OPTION_LEAD) {
            let command = GitCommand::from_str(word).ok()?;
            return command.changes_tree(&args[index + 1..]).then_some(dir);
        }
    }
    None
}

/// Whether a `gh` with `args` acts on the local checkout, when it is a
/// guarded `gh pr` subcommand that does more than read: not when it names
/// another repository with `-R` or `--repo`.
fn guarded_pr(args: &[String]) -> Option<bool> {
    let mut positional = Vec::new();
    let mut local = true;
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if GH_VALUE_OPTIONS.contains(&word.as_str()) {
            words.next();
            local = false;
        } else if word.starts_with(GH_REPO_ASSIGNED) {
            local = false;
        } else if !word.starts_with(OPTION_LEAD) {
            positional.push(word.as_str());
        }
    }
    let guarded = !is_read_only(args)
        && matches!(positional.as_slice(), [GH_PR, command, ..] if PrCommand::from_str(command).is_ok());
    guarded.then_some(local)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The folder every test command runs in.
    const CWD: &str = "/work/shop";
    /// The home folder of every test command.
    const HOME: &str = "/home/me";

    /// What `command` run in [`CWD`] guards.
    fn dirs(command: &str) -> Vec<Option<PathBuf>> {
        guarded_dirs(command, Path::new(CWD), Some(Path::new(HOME)))
    }

    /// `paths` as guarded folders.
    fn folders(paths: &[&str]) -> Vec<Option<PathBuf>> {
        paths.iter().map(|path| Some(PathBuf::from(path))).collect()
    }

    /// Commands that change or publish a tree are found; reading ones are
    /// not; a `gh pr` naming another repository touches no local checkout.
    #[test]
    fn tree_changing_commands_are_found_and_reading_ones_are_not() {
        for changing in [
            "git commit -m 'a; b'",
            "git commit --allow-empty-message -m ''",
            "git push origin HEAD",
            "git revert HEAD",
            "git \\\n  push origin main",
            "gh pr create --fill",
            "gh pr merge 12 --squash",
            "gh pr checkout 12",
        ] {
            assert_eq!(dirs(changing), folders(&[CWD]), "{changing}");
        }
        for elsewhere in [
            "gh -R owner/repo pr create",
            "gh pr --repo o/r merge 12",
            "gh pr merge 12 --repo=o/r",
        ] {
            assert_eq!(dirs(elsewhere), vec![None], "{elsewhere}");
        }
        for reading in [
            "git status",
            "git log --oneline -3",
            "git stash list",
            "git push --dry-run",
            "git clean -n",
            "git clean -nd",
            "git rm -rn dir",
            "git commit --help",
            "git worktree remove ../w",
            "gh pr view 12",
            "gh pr create --help",
            "gh -R pr/create pr view",
            "echo 'git push'",
            "cargo test",
            "",
        ] {
            assert!(dirs(reading).is_empty(), "{reading}");
        }
    }

    /// The folder follows `git -C`, `cd` with or without options or a
    /// folder, `~` and `$HOME`, and an option value is never read as the
    /// subcommand.
    #[test]
    fn the_folder_follows_cd_git_c_and_home() {
        assert_eq!(
            dirs("git -C ../api switch main"),
            folders(&["/work/shop/../api"])
        );
        assert_eq!(dirs("cd /work/api && git pull"), folders(&["/work/api"]));
        assert_eq!(dirs("cd ~/api && git push"), folders(&["/home/me/api"]));
        assert_eq!(dirs("git -C $HOME/api push"), folders(&["/home/me/api"]));
        assert_eq!(dirs("cd ${HOME} && git push"), folders(&[HOME]));
        assert_eq!(dirs("cd && git push"), folders(&[HOME]));
        assert_eq!(dirs("cd -P /work/api && git push"), folders(&["/work/api"]));
        assert_eq!(dirs("git -c core.pager=cat commit"), folders(&[CWD]));
        assert!(dirs("git -c push status").is_empty());
    }

    /// Every guarded folder is found once, in order, however many commands
    /// run in it.
    #[test]
    fn every_guarded_folder_is_found_once() {
        assert_eq!(
            dirs("git -C /work/a commit -m x && git -C /work/b push && git -C /work/a push"),
            folders(&["/work/a", "/work/b"])
        );
        assert_eq!(
            dirs("cd /tmp && git commit; cd /work/shop && git push"),
            folders(&["/tmp", CWD])
        );
    }

    /// Leading assignments, shell keywords, a full path and a pipeline
    /// still find the command.
    #[test]
    fn chained_commands_are_found() {
        for command in [
            "GIT_EDITOR=true git rebase --continue",
            "/usr/bin/git stash",
            "git status; git commit -am x",
            "echo done | git cherry-pick abc",
            "if true; then git commit -am x; fi",
            "for f in a; do git rm $f; done",
            "{ git push; }",
            "! git push",
        ] {
            assert_eq!(dirs(command), folders(&[CWD]), "{command}");
        }
    }

    /// Comments are not commands, and nothing past a heredoc is read, so
    /// no body line is taken for a command and two heredocs on one line
    /// read safely.
    #[test]
    fn comments_and_heredocs_are_not_read() {
        assert!(dirs("echo hi # git push").is_empty());
        assert_eq!(dirs("echo hi # don't\ngit push"), folders(&[CWD]));
        assert!(dirs("# git push\necho hi").is_empty());
        assert!(dirs("cat <<EOF > notes.md\ngit reset --hard\nEOF").is_empty());
        assert!(dirs("cat <<A <<B\nx\nA").is_empty());
        assert!(dirs("a=$((1<<2)); b=$((1<<3))\ngit commit -m x").is_empty());
        assert_eq!(dirs("git commit -F- <<EOF\nmsg\nEOF"), folders(&[CWD]));
        assert_eq!(dirs("echo a#b && git push"), folders(&[CWD]));
        assert_eq!(dirs("git commit -m 'a <<b'"), folders(&[CWD]));
    }

    /// An unclosed quote leaves its command unread rather than misread.
    #[test]
    fn an_unclosed_quote_is_skipped() {
        assert!(dirs("git commit -m 'oops").is_empty());
    }
}

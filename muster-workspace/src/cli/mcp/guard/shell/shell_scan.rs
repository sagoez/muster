use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::cli::mcp::guard::shell::{git_command::GitCommand, pr_command::PrCommand};

/// Characters that end one simple command and start the next, outside quotes.
const SEPARATORS: [char; 6] = [';', '&', '|', '\n', '(', ')'];
/// Ends a line, and with it a comment or a heredoc's opening line.
const NEWLINE: char = '\n';
/// Opens and closes a single-quoted word.
const SINGLE_QUOTE: char = '\'';
/// Opens and closes a double-quoted word.
const DOUBLE_QUOTE: char = '"';
/// Escapes the next character outside single quotes.
const BACKSLASH: char = '\\';
/// Starts a comment at the start of a word.
const COMMENT: char = '#';
/// Opens a redirection, doubled for a heredoc and tripled for a here-string.
const REDIRECT_IN: char = '<';
/// Marks a heredoc whose body lines may be indented with tabs.
const HEREDOC_DASH: char = '-';
/// Joins a variable to its value in a leading assignment.
const ASSIGN: char = '=';
/// The one punctuation a variable name may hold.
const NAME_PUNCTUATION: char = '_';
/// Starts an option word.
const OPTION_LEAD: char = '-';
/// Commands that run the rest of their words, past their options and
/// counts, as a command.
const WRAPPERS: [&str; 9] = [
    "command", "env", "exec", "nohup", "time", "sudo", "nice", "timeout", "xargs",
];
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
/// git itself.
const GIT: &str = "git";
/// git's flag naming the folder it runs in.
const GIT_DIR_FLAG: &str = "-C";
/// git's option naming the working tree.
const WORK_TREE_OPTION: &str = "--work-tree";
/// The variable naming git's working tree.
const GIT_WORK_TREE_ENV: &str = "GIT_WORK_TREE";
/// The variable naming git's repository folder.
const GIT_DIR_ENV: &str = "GIT_DIR";
/// The repository folder inside a working tree.
const DOT_GIT: &str = ".git";
/// git's options before the subcommand that take the next word as a value.
const GIT_VALUE_OPTIONS: [&str; 6] = [
    GIT_DIR_FLAG,
    "-c",
    "--git-dir",
    WORK_TREE_OPTION,
    "--namespace",
    "--config-env",
];
/// GitHub's CLI.
const GH: &str = "gh";
/// gh's options that take the next word as a value.
const GH_VALUE_OPTIONS: [&str; 2] = ["-R", "--repo"];
/// gh's pull request command group.
const GH_PR: &str = "pr";

/// The folders the git and gh commands of `command`, run in `cwd`, change
/// or publish a checkout in, each once, in order; none when it runs no such
/// command. It reads command by command, following `cd`, `git -C`,
/// `--work-tree`, `GIT_WORK_TREE` and `GIT_DIR`, leading assignments and a
/// few wrappers and keywords, reading `~` and `$HOME` as `home`, and skips
/// comments and heredoc bodies. A best effort, not a shell parser.
pub fn guarded_dirs(command: &str, cwd: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let mut dir = cwd.to_path_buf();
    let mut found: Vec<PathBuf> = Vec::new();
    for segment in segments(command) {
        let Some(words) = shlex::split(&segment) else {
            continue;
        };
        let (assignments, words) = command_words(&words);
        let Some((program, args)) = words.split_first() else {
            continue;
        };
        let guarded = match program_name(program) {
            CD => {
                if let Some(target) = args.first() {
                    dir = joined(&dir, target, home);
                }
                None
            },
            GIT => git_dir(args, &tree_of(assignments, &dir, home), home),
            GH => is_guarded_pr(args).then(|| dir.clone()),
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
/// leaving out comments and the bodies of heredocs.
fn segments(script: &str) -> Vec<String> {
    let chars: Vec<char> = script.chars().collect();
    let mut segments = vec![String::new()];
    let mut heredocs: Vec<String> = Vec::new();
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
            if chars.get(at + 1) == Some(&REDIRECT_IN) {
                at += 2;
            } else {
                let (delimiter, after) = heredoc_delimiter(&chars, at + 1);
                heredocs.extend(delimiter);
                at = after;
            }
            continue;
        } else if !quoted && SEPARATORS.contains(&character) {
            if character == NEWLINE && !heredocs.is_empty() {
                at = past_bodies(&chars, at, &heredocs);
                heredocs.clear();
            }
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

/// The delimiter of the heredoc whose `<<` ends just before `at`, its quotes
/// dropped, and where the redirection ends; no delimiter when none follows.
fn heredoc_delimiter(chars: &[char], mut at: usize) -> (Option<String>, usize) {
    if chars.get(at) == Some(&HEREDOC_DASH) {
        at += 1;
    }
    while chars
        .get(at)
        .is_some_and(|next| *next != NEWLINE && next.is_whitespace())
    {
        at += 1;
    }
    let mut delimiter = String::new();
    while let Some(&next) = chars.get(at) {
        if next.is_whitespace() || SEPARATORS.contains(&next) || next == REDIRECT_IN {
            break;
        }
        if next != SINGLE_QUOTE && next != DOUBLE_QUOTE && next != BACKSLASH {
            delimiter.push(next);
        }
        at += 1;
    }
    ((!delimiter.is_empty()).then_some(delimiter), at)
}

/// Where the bodies of `heredocs`, starting at `at` in `chars`, end: past
/// the line holding the last one's delimiter, or the end.
fn past_bodies(chars: &[char], mut at: usize, heredocs: &[String]) -> usize {
    for delimiter in heredocs {
        loop {
            let start = at;
            while chars.get(at).is_some_and(|next| *next != NEWLINE) {
                at += 1;
            }
            let line: String = chars[start..at.min(chars.len())].iter().collect();
            at += 1;
            if line.trim() == delimiter || at > chars.len() {
                break;
            }
        }
    }
    at.min(chars.len())
}

/// The leading variable assignments of `words`, and the words past them and
/// past wrapper commands, their options and counts, and shell keywords.
fn command_words(words: &[String]) -> (Vec<&String>, &[String]) {
    let mut assignments = Vec::new();
    let mut rest = words;
    let mut wrapped = false;
    while let Some((first, tail)) = rest.split_first() {
        if is_assignment(first) {
            assignments.push(first);
        } else if WRAPPERS.contains(&program_name(first)) || KEYWORDS.contains(&first.as_str()) {
            wrapped = true;
        } else if !(wrapped && (first.starts_with(OPTION_LEAD) || is_count(first))) {
            break;
        }
        rest = tail;
    }
    (assignments, rest)
}

/// Whether `word` is a count or a duration a wrapper takes, as `30` or `5s`.
fn is_count(word: &str) -> bool {
    word.chars()
        .next()
        .is_some_and(|first| first.is_ascii_digit())
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

/// The folder a git run in `dir` works in, as the `GIT_WORK_TREE` or
/// `GIT_DIR` among `assignments` moves it.
fn tree_of(assignments: Vec<&String>, dir: &Path, home: Option<&Path>) -> PathBuf {
    let value = |name: &str| {
        assignments
            .iter()
            .find_map(|word| word.strip_prefix(name)?.strip_prefix(ASSIGN))
    };
    if let Some(tree) = value(GIT_WORK_TREE_ENV) {
        return joined(dir, tree, home);
    }
    value(GIT_DIR_ENV)
        .map(|git_dir| joined(dir, git_dir, home))
        .filter(|git_dir| git_dir.file_name().is_some_and(|name| name == DOT_GIT))
        .and_then(|git_dir| git_dir.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| dir.to_path_buf())
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

/// The folder a `git` with `args`, run in `dir`, changes or publishes,
/// when its subcommand does.
fn git_dir(args: &[String], dir: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let mut dir = dir.to_path_buf();
    let mut words = args.iter().enumerate();
    while let Some((index, word)) = words.next() {
        if let Some(tree) = word
            .strip_prefix(WORK_TREE_OPTION)
            .and_then(|rest| rest.strip_prefix(ASSIGN))
        {
            dir = joined(&dir, tree, home);
        } else if GIT_VALUE_OPTIONS.contains(&word.as_str()) {
            let (_, value) = words.next()?;
            if word == GIT_DIR_FLAG || word == WORK_TREE_OPTION {
                dir = joined(&dir, value, home);
            }
        } else if !word.starts_with(OPTION_LEAD) {
            let command = GitCommand::from_str(word).ok()?;
            return command.changes_tree(&args[index + 1..]).then_some(dir);
        }
    }
    None
}

/// Whether a `gh` with `args` is a guarded `gh pr` subcommand.
fn is_guarded_pr(args: &[String]) -> bool {
    let mut positional = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if GH_VALUE_OPTIONS.contains(&word.as_str()) {
            words.next();
        } else if !word.starts_with(OPTION_LEAD) {
            positional.push(word.as_str());
        }
    }
    matches!(positional.as_slice(), [GH_PR, command, ..] if PrCommand::from_str(command).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The folder every test command runs in.
    const CWD: &str = "/work/shop";
    /// The home folder of every test command.
    const HOME: &str = "/home/me";

    /// The folders `command` run in [`CWD`] guards.
    fn dirs(command: &str) -> Vec<PathBuf> {
        guarded_dirs(command, Path::new(CWD), Some(Path::new(HOME)))
    }

    /// `paths` as folders.
    fn folders(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    /// Commands that change or publish a tree are found; reading ones are not.
    #[test]
    fn tree_changing_commands_are_found_and_reading_ones_are_not() {
        for changing in [
            "git commit -m 'a; b'",
            "git push origin HEAD",
            "git revert HEAD",
            "git worktree remove ../w",
            "gh pr create --fill",
            "gh pr merge 12 --squash",
            "gh pr checkout 12",
            "gh -R owner/repo pr create",
            "gh pr --repo o/r merge 12",
        ] {
            assert_eq!(dirs(changing), folders(&[CWD]), "{changing}");
        }
        for reading in [
            "git status",
            "git log --oneline -3",
            "git stash list",
            "git push --dry-run",
            "git worktree add ../w",
            "gh pr view 12",
            "gh -R pr/create pr view",
            "echo 'git push'",
            "cargo test",
            "",
        ] {
            assert!(dirs(reading).is_empty(), "{reading}");
        }
    }

    /// The folder follows `git -C`, `cd` and absolute paths, and an option
    /// value is never read as the subcommand.
    #[test]
    fn the_folder_follows_cd_and_git_c() {
        assert_eq!(
            dirs("git -C ../api switch main"),
            folders(&["/work/shop/../api"])
        );
        assert_eq!(dirs("cd /work/api && git pull"), folders(&["/work/api"]));
        assert_eq!(dirs("git -c core.pager=cat commit"), folders(&[CWD]));
        assert!(dirs("git -c push status").is_empty());
    }

    /// `~` and `$HOME` read as the home folder, and the working tree git is
    /// pointed at is the folder.
    #[test]
    fn home_and_work_trees_are_followed() {
        assert_eq!(dirs("cd ~/api && git push"), folders(&["/home/me/api"]));
        assert_eq!(dirs("git -C $HOME/api push"), folders(&["/home/me/api"]));
        assert_eq!(dirs("cd ${HOME} && git push"), folders(&[HOME]));
        assert_eq!(dirs("git --work-tree=/w/api commit"), folders(&["/w/api"]));
        assert_eq!(dirs("git --work-tree /w/api commit"), folders(&["/w/api"]));
        assert_eq!(
            dirs("GIT_WORK_TREE=/w/api git commit"),
            folders(&["/w/api"])
        );
        assert_eq!(dirs("GIT_DIR=/w/api/.git git commit"), folders(&["/w/api"]));
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

    /// Leading assignments, wrappers with their options and counts, shell
    /// keywords, a full path and a pipeline still find the command.
    #[test]
    fn wrapped_and_chained_commands_are_found() {
        for command in [
            "GIT_EDITOR=true git rebase --continue",
            "env FOO=1 /usr/bin/git stash",
            "git status; git commit -am x",
            "echo done | git cherry-pick abc",
            "if true; then git commit -am x; fi",
            "for f in a; do git rm $f; done",
            "{ git push; }",
            "! git push",
            "sudo git push",
            "timeout 30 git push",
            "env -i git push",
            "time -p git push",
        ] {
            assert_eq!(dirs(command), folders(&[CWD]), "{command}");
        }
    }

    /// Comments and heredoc bodies are not commands, but what follows a
    /// heredoc is.
    #[test]
    fn comments_and_heredoc_bodies_are_skipped() {
        assert!(dirs("echo hi # git push").is_empty());
        assert_eq!(dirs("echo hi # don't\ngit push"), folders(&[CWD]));
        assert!(dirs("# git push\necho hi").is_empty());
        assert!(dirs("cat <<EOF > notes.md\ngit reset --hard\nEOF").is_empty());
        assert!(dirs("cat <<-'END'\n\tgit push\n\tEND\necho ok").is_empty());
        assert_eq!(
            dirs("cat <<EOF > notes.md\ngit reset --hard\nEOF\ngit commit -am x"),
            folders(&[CWD])
        );
        assert_eq!(dirs("echo a#b && git push"), folders(&[CWD]));
        assert!(dirs("cat <<< 'git push'").is_empty());
    }

    /// An unclosed quote leaves its command unread rather than misread.
    #[test]
    fn an_unclosed_quote_is_skipped() {
        assert!(dirs("git commit -m 'oops").is_empty());
    }
}

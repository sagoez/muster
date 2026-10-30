//! The prompt hook `muster shell-init` prints for each shell.

use std::path::Path;

use super::{HookScriptError, HookShell};
use crate::constants::{MUSTER_OPEN_PROGRAM_ENV, MUSTER_SHELL_ENV};

/// The hidden subcommand the hook runs at every prompt to report its shell.
pub const SHELL_REPORT_SUBCOMMAND: &str = "__shell-report";

/// Where a template takes the muster binary's path.
const MUSTER_BIN_PLACEHOLDER: &str = "@MUSTER_BIN@";
/// Where a template takes the folder its notes are read from.
const NOTES_DIR_PLACEHOLDER: &str = "@MUSTER_NOTES@";
/// Where a template takes the variable naming the hooked shell's pid.
const SHELL_ENV_PLACEHOLDER: &str = "@MUSTER_SHELL_ENV@";
/// Where a template takes the report subcommand.
const REPORT_PLACEHOLDER: &str = "@MUSTER_REPORT@";
/// Where a template takes the variable set only in a pane `open` made.
const OPEN_ENV_PLACEHOLDER: &str = "@MUSTER_OPEN_ENV@";

/// Punctuation every hooked shell reads bare in a word, besides letters and
/// digits.
const BARE_SAFE: &str = "_-./:@+=,";
/// The single quote that opens and closes a quoted run.
const QUOTE: char = '\'';
/// A single quote escaped, outside quotes in bash and zsh, inside in fish.
const ESCAPED_QUOTE: &str = "\\'";
/// A backslash, which fish reads as an escape inside single quotes.
const BACKSLASH: char = '\\';
/// A backslash escaped inside fish single quotes.
const ESCAPED_BACKSLASH: &str = "\\\\";
/// The character no shell word can carry.
const NUL: char = '\0';

/// The bash hook, before its placeholders are filled.
const BASH_TEMPLATE: &str = include_str!("scripts/muster.bash");
/// The zsh hook, before its placeholders are filled.
const ZSH_TEMPLATE: &str = include_str!("scripts/muster.zsh");
/// The fish hook, before its placeholders are filled.
const FISH_TEMPLATE: &str = include_str!("scripts/muster.fish");

/// The hook for `shell`, running `binary` and reading notes from `notes_dir`.
///
/// # Errors
/// Returns [`HookScriptError`] when either path is not UTF-8 or holds a nul
/// byte.
pub fn hook_script(
    shell: HookShell,
    binary: &Path,
    notes_dir: &Path,
) -> Result<String, HookScriptError> {
    let template = match shell {
        HookShell::Bash => BASH_TEMPLATE,
        HookShell::Zsh => ZSH_TEMPLATE,
        HookShell::Fish => FISH_TEMPLATE,
    };
    Ok(template
        .replace(MUSTER_BIN_PLACEHOLDER, &quoted(shell, binary)?)
        .replace(NOTES_DIR_PLACEHOLDER, &quoted(shell, notes_dir)?)
        .replace(SHELL_ENV_PLACEHOLDER, MUSTER_SHELL_ENV)
        .replace(REPORT_PLACEHOLDER, SHELL_REPORT_SUBCOMMAND)
        .replace(OPEN_ENV_PLACEHOLDER, MUSTER_OPEN_PROGRAM_ENV))
}

/// `path` as one word of `shell`, as it is when every character is safe
/// bare.
///
/// # Errors
/// Returns [`HookScriptError`] when `path` is not UTF-8 or holds a nul byte.
fn quoted(shell: HookShell, path: &Path) -> Result<String, HookScriptError> {
    let text = path.to_str().ok_or_else(|| HookScriptError::NotUtf8 {
        path: path.to_path_buf(),
    })?;
    if text.contains(NUL) {
        return Err(HookScriptError::Unquotable {
            path: path.to_path_buf(),
        });
    }
    if text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || BARE_SAFE.contains(c))
    {
        return Ok(text.to_string());
    }
    Ok(match shell {
        HookShell::Bash | HookShell::Zsh => posix_quoted(text),
        HookShell::Fish => fish_quoted(text),
    })
}

/// `text` in POSIX single quotes for bash and zsh, each quote in it written
/// outside them as `\'`. Two quotes never meet inside a quoted run, so zsh
/// with `RC_QUOTES` reads it the same.
fn posix_quoted(text: &str) -> String {
    let mut word = String::new();
    let mut open = false;
    for c in text.chars() {
        if c == QUOTE {
            if open {
                word.push(QUOTE);
                open = false;
            }
            word.push_str(ESCAPED_QUOTE);
        } else {
            if !open {
                word.push(QUOTE);
                open = true;
            }
            word.push(c);
        }
    }
    if open {
        word.push(QUOTE);
    }
    word
}

/// `text` in fish single quotes, where a backslash and a quote are the only
/// characters escaped.
fn fish_quoted(text: &str) -> String {
    let escaped = text
        .replace(BACKSLASH, ESCAPED_BACKSLASH)
        .replace(QUOTE, ESCAPED_QUOTE);
    format!("{QUOTE}{escaped}{QUOTE}")
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsStr,
        fs,
        os::unix::{ffi::OsStrExt as _, fs::PermissionsExt as _},
        path::PathBuf,
        process::{Command, Output},
        thread,
        time::{Duration, Instant},
    };

    use strum::IntoEnumIterator;
    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::clock::nanos_now,
        domain::{
            occupancy::{LoadId, LoadStart},
            value::{EpochNanos, Pid},
        },
    };

    /// The binary path the snapshots are taken with.
    const SNAPSHOT_BINARY: &str = "/usr/local/bin/muster";
    /// The notes folder the snapshots are taken with.
    const SNAPSHOT_NOTES: &str = "/home/user/.local/state/muster/shells";
    /// A binary path no shell word can carry unquoted, with a caret, two
    /// quotes in a row and a backslash besides.
    const AWKWARD_BINARY: &str = "/tmp/a b^x/it's ''q\\/muster";
    /// What every placeholder starts with.
    const PLACEHOLDER_MARK: &str = "@MUSTER_";
    /// The folder, under a scratch dir, a test's fake muster binary lives in.
    const AWKWARD_DIR: &str = "a b^c/it's \"q\" $x";
    /// What a zsh test runs first, on a line of its own so it holds before
    /// the next line is read, to read words as a user with `RC_QUOTES`
    /// would.
    const RC_QUOTES_ON: &str = "setopt rc_quotes\n";
    /// The fake muster binary's file name.
    const FAKE_BINARY: &str = "muster";
    /// The file the fake binary appends each call's arguments to.
    const CALLS_FILE: &str = "calls";
    /// The fake muster binary: one line of arguments per call.
    const FAKE_BINARY_SCRIPT: &str = "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"${0%/*}/calls\"\n";
    /// Mode of the fake binary: runnable by its owner.
    const EXECUTABLE_MODE: u32 = 0o700;
    /// The script file a test loads the hook from.
    const SCRIPT_FILE: &str = "hook";
    /// The notes folder under a test's scratch dir.
    const NOTES_DIR: &str = "shells";
    /// How long a test waits for the detached report to run.
    const REPORT_WAIT: Duration = Duration::from_secs(10);
    /// How long a test keeps watching for a second, unwanted report.
    const SETTLE: Duration = Duration::from_millis(300);
    /// Between looks for the report.
    const POLL: Duration = Duration::from_millis(20);
    /// The status the driver's command before the prompt ends with.
    const KEPT_STATUS: &str = "status=1";
    /// What a driver prints when the hook was registered once.
    const ONE_HOOK: &str = "hooks=1";
    /// What starts the line a driver prints the exported shell pid on.
    const PID_PREFIX: &str = "pid=";
    /// What the time the shell arrived in its folder follows, when the shell
    /// can read its clock.
    const ARRIVED_FLAG: &str = " --arrived ";
    /// What starts the line a driver prints the hook's load start and the
    /// pid it spawned on.
    const LOAD_PREFIX: &str = "load=";
    /// Lines the drivers leave as two note files, in seq order; the second
    /// holds what a shell could expand and an escape sanitizing writes.
    const NOTE_LINES: [&str; 2] = ["first line", "second $line \\x1b"];
    /// What a probe runs to learn whether a shell runs here.
    const PROBE_SCRIPT: &str = "exit 0";
    /// Digits of a load start in nanoseconds from 2001 to 2286.
    const NANOSECOND_DIGITS: usize = 19;

    /// Each shell's driver: loads the hook twice, leaves two notes for this
    /// shell, fails a command, runs one prompt, then prints the status the
    /// prompt kept, how often the hook is registered and the pid it exported.
    fn driver(shell: HookShell) -> &'static str {
        match shell {
            HookShell::Bash => concat!(
                ". \"$1\"; . \"$1\"\n",
                "printf '%s\\n' 'second $line \\x1b' > \"$2/$MUSTER_SHELL_PID-00000000000000000002.note\"\n",
                "printf '%s\\n' 'first line' > \"$2/$MUSTER_SHELL_PID-00000000000000000001.note\"\n",
                "false\n",
                "for hook in \"${PROMPT_COMMAND[@]}\"; do eval \"$hook\"; done\n",
                "echo \"status=$?\"\n",
                "hooks=0; for hook in \"${PROMPT_COMMAND[@]}\"; do\n",
                "  [[ $hook == __muster_prompt ]] && hooks=$((hooks + 1))\n",
                "done; echo \"hooks=$hooks\"\n",
                "echo \"pid=$MUSTER_SHELL_PID\"\n",
                "echo \"load=$__muster_load $__muster_load_pid\"\n",
            ),
            HookShell::Zsh => concat!(
                ". \"$1\"; . \"$1\"\n",
                "printf '%s\\n' 'second $line \\x1b' > \"$2/$MUSTER_SHELL_PID-00000000000000000002.note\"\n",
                "printf '%s\\n' 'first line' > \"$2/$MUSTER_SHELL_PID-00000000000000000001.note\"\n",
                "false\n",
                "for hook in $precmd_functions; do $hook; done\n",
                "echo \"status=$?\"\n",
                "hooks=(${(M)precmd_functions:#__muster_precmd}); echo \"hooks=${#hooks}\"\n",
                "echo \"pid=$MUSTER_SHELL_PID\"\n",
                "echo \"load=$__muster_load $__muster_load_pid\"\n",
            ),
            HookShell::Fish => concat!(
                "source $argv[1]; source $argv[1]\n",
                "printf '%s\\n' 'second $line \\x1b' > $argv[2]/$MUSTER_SHELL_PID-00000000000000000002.note\n",
                "printf '%s\\n' 'first line' > $argv[2]/$MUSTER_SHELL_PID-00000000000000000001.note\n",
                "false\n",
                "__muster_prompt\n",
                "echo \"status=$status\"\n",
                "echo hooks=(functions --handlers-type generic | string match -e __muster_prompt | count)\n",
                "echo \"pid=$MUSTER_SHELL_PID\"\n",
                "echo \"load=$__muster_load $__muster_load_pid\"\n",
            ),
        }
    }

    /// Runs `shell` with no user config on `script`, as an interactive
    /// shell when `interactive`, passing `args`; none when the shell is not
    /// installed, which is reported.
    fn run(shell: HookShell, interactive: bool, script: &str, args: &[&Path]) -> Option<Output> {
        run_in(shell, interactive, script, args, &[])
    }

    /// Runs `shell` as [`run`] does, with `env` set besides.
    fn run_in(
        shell: HookShell,
        interactive: bool,
        script: &str,
        args: &[&Path],
        env: &[(&str, &str)],
    ) -> Option<Output> {
        let mut command = Command::new(shell.to_string());
        match shell {
            HookShell::Bash => command.args(["--norc", "--noprofile"]),
            HookShell::Zsh => command.arg("-f"),
            HookShell::Fish => command.arg("--no-config"),
        };
        if interactive {
            command.arg("-i");
        }
        command.arg("-c").arg(script);
        if shell != HookShell::Fish {
            command.arg(shell.to_string());
        }
        command
            .args(args)
            .env_remove(MUSTER_SHELL_ENV)
            .env_remove(MUSTER_OPEN_PROGRAM_ENV)
            .envs(env.iter().copied());
        shell_runs(shell, "check").then(|| {
            command
                .output()
                .unwrap_or_else(|error| panic!("cannot run {shell}: {error}"))
        })
    }

    /// Whether `shell` runs here at all, probed with an empty command; a
    /// shell that is missing, or a shim that refuses to run it, has its
    /// `check` skipped with a printed reason rather than failed.
    fn shell_runs(shell: HookShell, check: &str) -> bool {
        let runs = Command::new(shell.to_string())
            .args(["-c", PROBE_SCRIPT])
            .env_remove(MUSTER_SHELL_ENV)
            .output()
            .is_ok_and(|output| output.status.success());
        if !runs {
            eprintln!("{shell} is not installed or does not run: its {check} is skipped");
        }
        runs
    }

    /// The hook for `shell`, written under `scratch`, running a fake muster
    /// in an awkwardly named folder; returns the script, the notes folder
    /// and the fake's calls file.
    fn installed_hook(shell: HookShell, scratch: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let bin_dir = scratch.join(AWKWARD_DIR);
        fs::create_dir_all(&bin_dir).unwrap();
        let binary = bin_dir.join(FAKE_BINARY);
        fs::write(&binary, FAKE_BINARY_SCRIPT).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(EXECUTABLE_MODE)).unwrap();
        let notes = scratch.join(NOTES_DIR);
        fs::create_dir_all(&notes).unwrap();
        let script = scratch.join(SCRIPT_FILE);
        fs::write(&script, hook_script(shell, &binary, &notes).unwrap()).unwrap();
        (script, notes, bin_dir.join(CALLS_FILE))
    }

    /// The fake binary's calls once the first has landed and no other
    /// followed within [`SETTLE`].
    fn calls(file: &Path) -> Vec<String> {
        let deadline = Instant::now() + REPORT_WAIT;
        while !file.exists() && Instant::now() < deadline {
            thread::sleep(POLL);
        }
        thread::sleep(SETTLE);
        fs::read_to_string(file)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn each_script_snapshot() {
        for shell in HookShell::iter() {
            let script =
                hook_script(shell, Path::new(SNAPSHOT_BINARY), Path::new(SNAPSHOT_NOTES)).unwrap();
            insta::assert_snapshot!(format!("hook_{shell}"), script);
        }
    }

    #[test]
    fn every_placeholder_is_filled() {
        for shell in HookShell::iter() {
            let script =
                hook_script(shell, Path::new(AWKWARD_BINARY), Path::new(SNAPSHOT_NOTES)).unwrap();
            assert!(!script.is_empty());
            assert!(!script.contains(PLACEHOLDER_MARK), "{shell}: {script}");
            assert!(script.contains(SHELL_REPORT_SUBCOMMAND));
            assert!(script.contains(MUSTER_SHELL_ENV));
        }
    }

    #[test]
    fn the_names_filled_in_as_is_need_no_quoting() {
        for shell in HookShell::iter() {
            for name in [
                MUSTER_SHELL_ENV,
                SHELL_REPORT_SUBCOMMAND,
                MUSTER_OPEN_PROGRAM_ENV,
            ] {
                assert_eq!(quoted(shell, Path::new(name)).unwrap(), name, "{shell}");
            }
        }
    }

    /// Security M1: an awkward path, with a caret and quotes, reads back
    /// whole in each shell, and in zsh with `RC_QUOTES` set too.
    #[test]
    fn a_binary_path_with_spaces_and_quotes_is_quoted_for_each_shell() {
        for shell in HookShell::iter() {
            let word = quoted(shell, Path::new(AWKWARD_BINARY)).unwrap();
            let script =
                hook_script(shell, Path::new(AWKWARD_BINARY), Path::new(SNAPSHOT_NOTES)).unwrap();
            assert!(script.contains(&word), "{shell}: {script}");
            let mut readings = vec![format!("printf '%s' {word}")];
            if shell == HookShell::Zsh {
                readings.push(format!("{RC_QUOTES_ON}x={word}; printf '%s' \"$x\""));
            }
            for reading in readings {
                let Some(output) = run(shell, false, &reading, &[]) else {
                    continue;
                };
                assert!(output.status.success(), "{shell}: {reading}: {output:?}");
                assert_eq!(
                    String::from_utf8_lossy(&output.stdout),
                    AWKWARD_BINARY,
                    "{shell}: {reading}"
                );
            }
        }
    }

    /// The zsh hook loaded with `RC_QUOTES` set, from an awkward binary and
    /// notes folder, keeps both paths whole.
    #[test]
    fn the_zsh_hook_keeps_awkward_paths_under_rc_quotes() {
        let scratch = TempDir::new().unwrap();
        let script = scratch.path().join(SCRIPT_FILE);
        fs::write(
            &script,
            hook_script(
                HookShell::Zsh,
                Path::new(AWKWARD_BINARY),
                Path::new(AWKWARD_BINARY),
            )
            .unwrap(),
        )
        .unwrap();
        let probe =
            format!("{RC_QUOTES_ON}. \"$1\"; printf '%s\\n' \"$__muster_bin\" \"$__muster_notes\"");

        let Some(output) = run(HookShell::Zsh, true, &probe, &[&script]) else {
            return;
        };

        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{AWKWARD_BINARY}\n{AWKWARD_BINARY}\n")
        );
    }

    /// The load id a driver printed as its start and the pid it spawned.
    fn load_id(printed: &str) -> LoadId {
        let (started, pid) = printed
            .split_once(' ')
            .unwrap_or_else(|| panic!("no load pid in {printed:?}"));
        assert_eq!(started.len(), NANOSECOND_DIGITS, "{printed:?}");
        LoadId::builder()
            .started(started.parse::<LoadStart>().unwrap())
            .pid(Pid::try_new(pid.parse().unwrap()).unwrap())
            .build()
    }

    /// CR-1: each shell's hook loaded again in the same shell, as an `exec`
    /// does, takes a load id greater than the one before: its start in
    /// nanoseconds, then the pid it spawned.
    #[test]
    fn a_reload_in_the_same_shell_takes_a_greater_load_id() {
        for shell in HookShell::iter() {
            let scratch = TempDir::new().unwrap();
            let (script, ..) = installed_hook(shell, scratch.path());
            let reload = match shell {
                HookShell::Bash | HookShell::Zsh => concat!(
                    ". \"$1\"; echo \"$__muster_load $__muster_load_pid\"\n",
                    "unset __muster_hook\n",
                    ". \"$1\"; echo \"$__muster_load $__muster_load_pid\"\n",
                ),
                HookShell::Fish => concat!(
                    "source $argv[1]; echo \"$__muster_load $__muster_load_pid\"\n",
                    "set -e __muster_hook\n",
                    "source $argv[1]; echo \"$__muster_load $__muster_load_pid\"\n",
                ),
            };

            let Some(output) = run(shell, true, reload, &[&script]) else {
                continue;
            };

            let stdout = String::from_utf8_lossy(&output.stdout);
            let loads: Vec<LoadId> = stdout.lines().map(load_id).collect();
            assert_eq!(loads.len(), 2, "{shell}: {output:?}");
            assert!(loads[1] > loads[0], "{shell}: {loads:?}");
        }
    }

    #[test]
    fn each_script_passes_its_shell_s_syntax_check() {
        let scratch = TempDir::new().unwrap();
        for shell in HookShell::iter() {
            let script = scratch.path().join(shell.to_string());
            fs::write(
                &script,
                hook_script(shell, Path::new(AWKWARD_BINARY), Path::new(SNAPSHOT_NOTES)).unwrap(),
            )
            .unwrap();
            let check = match shell {
                HookShell::Bash | HookShell::Zsh => "-n",
                HookShell::Fish => "--no-execute",
            };
            if !shell_runs(shell, "syntax check") {
                continue;
            }
            let output = Command::new(shell.to_string())
                .arg(check)
                .arg(&script)
                .output()
                .unwrap_or_else(|error| panic!("cannot run {shell}: {error}"));
            assert!(output.status.success(), "{shell}: {output:?}");
            assert!(output.stderr.is_empty(), "{shell}: {output:?}");
        }
    }

    #[test]
    fn a_prompt_prints_and_removes_the_notes_keeps_the_status_and_reports_once() {
        for shell in HookShell::iter() {
            let scratch = TempDir::new().unwrap();
            let (script, notes, calls_file) = installed_hook(shell, scratch.path());
            let before = nanos_now();
            let Some(output) = run(shell, true, driver(shell), &[&script, &notes]) else {
                continue;
            };
            let after = nanos_now();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let lines: Vec<&str> = stdout.lines().collect();
            let pid = lines
                .iter()
                .find_map(|line| line.strip_prefix(PID_PREFIX))
                .unwrap_or_else(|| panic!("{shell}: {output:?}"));
            let load = lines
                .iter()
                .find_map(|line| line.strip_prefix(LOAD_PREFIX))
                .unwrap_or_else(|| panic!("{shell}: {output:?}"));
            let id = load_id(load);
            assert_eq!(
                lines,
                [
                    NOTE_LINES[0],
                    NOTE_LINES[1],
                    KEPT_STATUS,
                    ONE_HOOK,
                    &format!("{PID_PREFIX}{pid}"),
                    &format!("{LOAD_PREFIX}{load}")
                ],
                "{shell}: {output:?}"
            );
            assert_eq!(fs::read_dir(&notes).unwrap().count(), 0, "{shell}");
            let made = calls(&calls_file);
            let [call] = &made[..] else {
                panic!("{shell}: one report: {made:?}");
            };
            let reported = format!(
                "{SHELL_REPORT_SUBCOMMAND} --pid {pid} --load {} --load-pid {} --seq 1",
                id.started(),
                id.pid()
            );
            let timed = call
                .strip_prefix(&reported)
                .unwrap_or_else(|| panic!("{shell}: {call}"));
            let arrived = timed
                .strip_prefix(ARRIVED_FLAG)
                .map(|arrived| arrived.parse::<EpochNanos>().unwrap());
            assert!(
                arrived.is_some() || shell != HookShell::Zsh,
                "{shell}: zsh reads its clock: {call}"
            );
            assert!(
                arrived.is_none_or(|arrived| (before..=after).contains(&arrived)),
                "{shell}: the first prompt's time: {call}"
            );
        }
    }

    #[test]
    fn a_non_interactive_shell_loads_nothing() {
        for shell in HookShell::iter() {
            let scratch = TempDir::new().unwrap();
            let (script, notes, _) = installed_hook(shell, scratch.path());
            let probe = match shell {
                HookShell::Bash | HookShell::Zsh => {
                    ". \"$1\"\necho \"${__muster_hook-none}${MUSTER_SHELL_PID-none}\""
                },
                HookShell::Fish => {
                    "source $argv[1]\nset -q __muster_hook; or set -q MUSTER_SHELL_PID; or echo nonenone"
                },
            };
            let Some(output) = run(shell, false, probe, &[&script, &notes]) else {
                continue;
            };
            assert!(output.status.success(), "{shell}: {output:?}");
            assert_eq!(String::from_utf8_lossy(&output.stdout), "nonenone\n");
        }
    }

    /// Concurrency M3: the shell of a pane `muster open` made loads no hook,
    /// so its prompts report nothing and it never occupies the project.
    #[test]
    fn the_shell_of_an_open_pane_reports_nothing() {
        for shell in HookShell::iter() {
            let scratch = TempDir::new().unwrap();
            let (script, notes, calls_file) = installed_hook(shell, scratch.path());
            let pane = [(MUSTER_OPEN_PROGRAM_ENV, "/usr/local/bin/muster")];
            let Some(output) = run_in(shell, true, driver(shell), &[&script, &notes], &pane) else {
                continue;
            };

            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(stdout.contains("hooks=0"), "{shell}: {output:?}");
            assert!(
                stdout.lines().any(|line| line == PID_PREFIX),
                "{shell}: {output:?}"
            );
            thread::sleep(SETTLE);
            assert!(!calls_file.exists(), "{shell}: {:?}", calls(&calls_file));
        }
    }

    #[test]
    fn a_path_that_is_not_utf8_is_refused_naming_it() {
        let binary = Path::new(OsStr::from_bytes(b"/tmp/\xffmuster"));
        let error = hook_script(HookShell::Bash, binary, Path::new(SNAPSHOT_NOTES)).unwrap_err();
        assert!(matches!(error, HookScriptError::NotUtf8 { .. }));
        assert!(error.to_string().contains("/tmp/"));
    }
}

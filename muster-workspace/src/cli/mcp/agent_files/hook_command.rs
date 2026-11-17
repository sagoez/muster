//! The command line muster's session-start hook runs, and how muster knows
//! its own hook by it: `mcp context` with the hook flag, which nothing but a
//! hook muster wrote passes, wrapped so it never fails a session.

use std::path::Path;

use muster_domain::constants::{APP_NAME, MCP_SUBCOMMAND};

use crate::cli::{
    failure::error::CliError,
    mcp::mcp_command::{CONTEXT_SUBCOMMAND, HOOK_FLAG},
    stack::shell_join,
};

/// Opens the group that keeps the hook quiet.
const QUIET_OPEN: &str = "{ ";
/// Closes it: whatever the run ends with, the hook succeeds, and anything
/// the shell or an old muster says goes nowhere.
const QUIET_CLOSE: &str = " || true; } 2>/dev/null";
/// Opens a long flag.
const LONG_FLAG: &str = "--";

/// The hook flag as it is typed.
fn hook_flag() -> String {
    format!("{LONG_FLAG}{HOOK_FLAG}")
}

/// The hook's command line running `binary`'s `mcp context` as a hook,
/// quoted for the shell the agent runs it in, and wrapped so that a binary
/// gone, or one too old to know `mcp context --hook`, ends it quietly with
/// success: a muster hook never shows an error or fails a session start.
///
/// # Errors
/// Returns [`CliError::InvalidCommand`] when `binary` cannot be a shell word.
pub fn context_command(binary: &str) -> Result<String, CliError> {
    let flag = hook_flag();
    let run = shell_join([binary, MCP_SUBCOMMAND, CONTEXT_SUBCOMMAND, &flag])?;
    Ok(format!("{QUIET_OPEN}{run}{QUIET_CLOSE}"))
}

/// Whether `command` is muster's hook: `mcp context` with the hook flag and
/// nothing else, run by the bare `muster` found on `PATH` or by an absolute
/// path to the binary whatever it is named, as [`context_command`] wraps it
/// or bare. A relative path such as `./muster`, which a cloned repository
/// could ship, and any other spelling, such as one wrapped in `env`, is
/// someone else's hook.
pub fn is_context_command(command: &str) -> bool {
    let run = command
        .strip_prefix(QUIET_OPEN)
        .and_then(|inner| inner.strip_suffix(QUIET_CLOSE))
        .unwrap_or(command);
    let Some(words) = shlex::split(run) else {
        return false;
    };
    match words.as_slice() {
        [binary, mcp, context, flag] => {
            mcp == MCP_SUBCOMMAND
                && context == CONTEXT_SUBCOMMAND
                && *flag == hook_flag()
                && (binary == APP_NAME || Path::new(binary).is_absolute())
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        process::{Command, Output},
    };

    use serde_json::Value;
    use tempfile::TempDir;

    use super::*;
    use crate::cli::mcp::hook_file::HookFile;

    /// The shell agents run hook commands in.
    const SHELL: &str = "sh";
    /// A muster too old to know `mcp context --hook`: it complains and
    /// exits 2, as a usage error does.
    const OLD_MUSTER: &str =
        "#!/bin/sh\necho \"error: unexpected argument '--hook'\" >&2\nexit 2\n";
    /// The stub's mode: its owner may run it.
    const RUNNABLE: u32 = 0o700;

    /// Muster's hook is known by its command and hook flag whatever the
    /// binary is named, wrapped or bare, and nothing else is taken for it.
    #[test]
    fn the_hook_is_known_by_its_command() {
        assert!(is_context_command("/opt/bin/muster mcp context --hook"));
        assert!(is_context_command(
            "/home/me/.cargo/bin/muster-nightly mcp context --hook"
        ));
        assert!(is_context_command(
            &context_command("/home/me/my tools/muster").unwrap()
        ));
        assert!(is_context_command(&context_command(APP_NAME).unwrap()));
        assert!(!is_context_command("/opt/bin/muster mcp context"));
        assert!(!is_context_command("muster mcp context --hook --extra"));
        assert!(!is_context_command("echo muster mcp context --hook"));
        assert!(!is_context_command("env muster mcp context --hook"));
        assert!(!is_context_command("./muster mcp context --hook"));
        assert!(!is_context_command(&context_command("./muster").unwrap()));
        assert!(!is_context_command("tools/muster mcp context --hook"));
        assert!(!is_context_command(
            "bash '/home/me/.claude/hooks/herdr.sh' session"
        ));
    }

    /// The hook is wrapped to succeed quietly, and a path with a space
    /// stays one word.
    #[test]
    fn the_hook_is_wrapped_and_paths_are_quoted() {
        assert_eq!(
            context_command(APP_NAME).unwrap(),
            "{ muster mcp context --hook || true; } 2>/dev/null"
        );
        assert_eq!(
            context_command("/home/me/my tools/muster").unwrap(),
            "{ '/home/me/my tools/muster' mcp context --hook || true; } 2>/dev/null"
        );
        assert!(matches!(
            context_command("/bad\0path"),
            Err(CliError::InvalidCommand)
        ));
    }

    /// The command a hook file holds once muster puts its hook running
    /// `binary` in it, run as an agent runs it.
    fn run_installed(scratch: &TempDir, binary: &str) -> Output {
        let file = HookFile::builder()
            .path(scratch.path().join("settings.json"))
            .private(false)
            .build();
        file.install(&context_command(binary).unwrap()).unwrap();
        let written: Value =
            serde_json::from_str(&fs::read_to_string(file.path()).unwrap()).unwrap();
        let command = written["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .to_owned();
        Command::new(SHELL).args(["-c", &command]).output().unwrap()
    }

    /// A hook whose binary is gone, or too old to know the hook flag, ends
    /// with success and says nothing at the session start.
    #[test]
    fn a_missing_or_old_binary_never_fails_the_session() {
        let scratch = TempDir::new().unwrap();
        let gone = scratch.path().join("gone").join("muster");
        let old = scratch.path().join("old-muster");
        fs::write(&old, OLD_MUSTER).unwrap();
        fs::set_permissions(&old, fs::Permissions::from_mode(RUNNABLE)).unwrap();

        for binary in [gone, old] {
            let ran = run_installed(&TempDir::new().unwrap(), binary.to_str().unwrap());
            assert!(ran.status.success(), "{}: {ran:?}", binary.display());
            assert!(ran.stdout.is_empty() && ran.stderr.is_empty(), "{ran:?}");
        }
    }
}

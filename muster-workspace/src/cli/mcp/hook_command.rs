//! The command line muster's session-start hook runs, and how muster knows
//! its own hook by it.

use std::path::Path;

use muster_domain::constants::{APP_NAME, MCP_SUBCOMMAND};

use crate::cli::{
    failure::error::CliError, mcp::mcp_command::CONTEXT_SUBCOMMAND, stack::shell_join,
};

/// Runs the bare `muster` only where it is installed, so a project's shared
/// hook never fails the session of a teammate without muster.
const GUARD_LEAD: &str = "if command -v";
/// Silences the guard's lookup.
const GUARD_QUIET: &str = ">/dev/null 2>&1; then";
/// Ends the guard.
const GUARD_END: &str = "; fi";

/// The hook's command line running `binary`'s `mcp context`, quoted for the
/// shell the agent runs it in. With `shared`, for a project's settings file
/// every clone reads, `binary` is the bare `muster` and runs only when it is
/// installed, so the hook ends quietly elsewhere.
///
/// # Errors
/// Returns [`CliError::InvalidCommand`] when `binary` cannot be a shell word.
pub fn context_command(binary: &str, shared: bool) -> Result<String, CliError> {
    let run = shell_join([binary, MCP_SUBCOMMAND, CONTEXT_SUBCOMMAND])?;
    if !shared {
        return Ok(run);
    }
    let word = shell_join([binary])?;
    Ok(format!(
        "{GUARD_LEAD} {word} {GUARD_QUIET} {run}{GUARD_END}"
    ))
}

/// Whether `command` is muster's hook: `mcp context` run by a binary named
/// `muster` wherever it lives, with nothing else, or the guarded form
/// [`context_command`] writes for a project. Any other spelling, such as
/// one wrapped in `env` or `exec`, is someone else's hook.
pub fn is_context_command(command: &str) -> bool {
    if context_command(APP_NAME, true).is_ok_and(|guarded| guarded == command) {
        return true;
    }
    let Some(words) = shlex::split(command) else {
        return false;
    };
    match words.as_slice() {
        [binary, mcp, context] => {
            mcp == MCP_SUBCOMMAND
                && context == CONTEXT_SUBCOMMAND
                && Path::new(binary)
                    .file_name()
                    .is_some_and(|name| name == APP_NAME)
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Muster's hook is known by its command wherever the binary lives, the
    /// project's guarded form too, and nothing else is taken for it.
    #[test]
    fn the_hook_is_known_by_its_command() {
        assert!(is_context_command("/opt/bin/muster mcp context"));
        assert!(is_context_command("muster mcp context"));
        assert!(is_context_command(
            &context_command("/home/me/my tools/muster", false).unwrap()
        ));
        assert!(is_context_command(
            &context_command(APP_NAME, true).unwrap()
        ));
        assert!(!is_context_command("muster mcp context --extra"));
        assert!(!is_context_command("/opt/bin/not-muster mcp context"));
        assert!(!is_context_command("echo muster mcp context"));
        assert!(!is_context_command("env muster mcp context"));
        assert!(!is_context_command(
            "bash '/home/me/.claude/hooks/herdr.sh' session"
        ));
    }

    /// The project's hook runs muster only where it is installed, and a
    /// path with a space stays one word.
    #[test]
    fn the_shared_hook_is_guarded_and_paths_are_quoted() {
        assert_eq!(
            context_command(APP_NAME, true).unwrap(),
            "if command -v muster >/dev/null 2>&1; then muster mcp context; fi"
        );
        assert_eq!(
            context_command("/home/me/my tools/muster", false).unwrap(),
            "'/home/me/my tools/muster' mcp context"
        );
        assert!(matches!(
            context_command("/bad\0path", false),
            Err(CliError::InvalidCommand)
        ));
    }
}

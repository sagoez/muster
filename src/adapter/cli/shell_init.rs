//! `muster shell-init <shell>`: the prompt hook a person loads from their
//! shell's rc file, and the line that loads it.

use std::path::Path;

use super::{cli_ports::CliPorts, error::CliError};
use crate::{
    adapter::{
        client::ClientError,
        shell::{HookShell, hook_script},
    },
    constants::APP_NAME,
};

/// The command that prints the hook.
pub const SHELL_INIT_SUBCOMMAND: &str = "shell-init";
/// Opens the bash and zsh line, which evaluates what the command prints.
const EVAL_OPEN: &str = "eval \"$(";
/// Closes it.
const EVAL_CLOSE: &str = ")\"";
/// Ends the fish line, which sources what the command prints.
const FISH_SOURCE: &str = " | source";

/// Prints the hook for `shell`, running `binary` at each prompt and reading
/// notes from the folder the ports' shell files keep them in.
///
/// # Errors
/// Returns [`CliError::Client`] when there is no state directory, or
/// [`CliError::Unhookable`] when a path cannot be written into the script.
pub fn shell_init(
    ports: &dyn CliPorts,
    shell: HookShell,
    binary: &Path,
) -> Result<String, CliError> {
    let notes_dir = ports
        .shell_registry()
        .map_err(ClientError::from)?
        .notes_dir();
    Ok(hook_script(shell, binary, &notes_dir)?)
}

/// The line a person adds to `shell`'s rc file to load the hook.
pub fn rc_line(shell: HookShell) -> String {
    let command = format!("{APP_NAME} {SHELL_INIT_SUBCOMMAND} {shell}");
    match shell {
        HookShell::Bash | HookShell::Zsh => format!("{EVAL_OPEN}{command}{EVAL_CLOSE}"),
        HookShell::Fish => format!("{command}{FISH_SOURCE}"),
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::{super::fake_ports::FakePorts, *};
    use crate::adapter::{client::FailureKind, store::test_support::fresh_store};

    /// Where the hook names the notes folder, in the snapshot.
    const NOTES_PLACEHOLDER: &str = "<notes>";
    /// The binary each hook in the snapshot runs.
    const BINARY: &str = "/opt/muster bin/muster";

    /// Each shell's hook runs the binary it is handed and reads the notes
    /// folder of the ports' shell files.
    #[test]
    fn shell_init_prints_each_shell_s_hook() {
        let (store, _scratch) = fresh_store();
        let ports = FakePorts::new(store);
        let notes = ports.shell_registry().unwrap().notes_dir();
        let notes = notes.to_str().unwrap();

        let hooks: Vec<String> = HookShell::iter()
            .map(|shell| {
                let script = shell_init(&ports, shell, Path::new(BINARY)).unwrap();
                assert!(script.contains(notes), "{shell}: {script}");
                format!("# {shell}\n{}", script.replace(notes, NOTES_PLACEHOLDER))
            })
            .collect();

        insta::assert_snapshot!(hooks.join("\n"));
    }

    /// A binary path no shell word can carry is refused, naming it.
    #[test]
    fn a_binary_no_script_can_name_is_refused() {
        let (store, _scratch) = fresh_store();
        let ports = FakePorts::new(store);

        let refused = shell_init(&ports, HookShell::Bash, Path::new("/opt/mu\0ster")).unwrap_err();

        assert!(matches!(refused, CliError::Unhookable(_)), "{refused}");
        assert_eq!(refused.kind(), FailureKind::InvalidInput);
    }

    /// bash and zsh evaluate the printed hook; fish sources it.
    #[test]
    fn each_shell_s_rc_line_loads_its_hook() {
        assert_eq!(
            rc_line(HookShell::Bash),
            r#"eval "$(muster shell-init bash)""#
        );
        assert_eq!(
            rc_line(HookShell::Zsh),
            r#"eval "$(muster shell-init zsh)""#
        );
        assert_eq!(rc_line(HookShell::Fish), "muster shell-init fish | source");
    }
}

use std::{ffi::OsString, iter};

use crate::{
    constants::{MUSTER_OPEN_ARG_PREFIX as ARG_ENV_PREFIX, MUSTER_OPEN_PROGRAM_ENV},
    domain::terminal_workspace::PaneCommand,
};

/// Leads the line, which keeps it out of the history of a shell that skips
/// space-led lines: fish always, bash with `HISTCONTROL=ignorespace` and zsh
/// with `HIST_IGNORE_SPACE`, neither of them by default.
const HISTORY_SKIP: &str = " ";
/// Opens the guard that runs the rest only where the program's variable is
/// set, the same in bash, zsh and fish 3.
const GUARD_OPEN: &str = "[ -n ";
/// Closes the guard.
const GUARD_CLOSE: &str = " ] && ";
/// Opens a quoted reference to a variable, the same in bash, zsh and fish.
const REFERENCE_OPEN: &str = "\"$";
/// Closes a quoted reference to a variable.
const REFERENCE_CLOSE: &str = "\"";
/// Separates the references.
const WORD_SEPARATOR: &str = " ";
/// Ends the pane's shell once the command exits 0, so the pane closes.
const EXIT_ON_SUCCESS: &str = " && exit";

/// The variables `command`'s words travel in, set on the new pane, and the
/// fixed line typed into its shell that runs `command` from them. No word is
/// ever typed, so nothing a word holds can be read as shell syntax. The line
/// is guarded on the program's variable, so a copy recalled from history in
/// a shell without it does nothing and never exits.
pub(super) fn typed_line(command: &PaneCommand) -> (Vec<(String, OsString)>, String) {
    let words = iter::once(command.program().as_os_str().to_os_string())
        .chain(command.args().iter().cloned());
    let env: Vec<(String, OsString)> = words
        .enumerate()
        .map(|(index, word)| (format!("{ARG_ENV_PREFIX}{index}"), word))
        .collect();
    let references: Vec<String> = env
        .iter()
        .map(|(name, _)| format!("{REFERENCE_OPEN}{name}{REFERENCE_CLOSE}"))
        .collect();
    let line = format!(
        "{HISTORY_SKIP}{GUARD_OPEN}{REFERENCE_OPEN}{MUSTER_OPEN_PROGRAM_ENV}{REFERENCE_CLOSE}\
         {GUARD_CLOSE}{}{EXIT_ON_SUCCESS}",
        references.join(WORD_SEPARATOR)
    );
    (env, line)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A command whose words would each break a typed shell line.
    fn hostile() -> PaneCommand {
        PaneCommand::builder()
            .program(PathBuf::from("/opt/my muster/muster"))
            .args(vec![
                OsString::from("--project"),
                OsString::from("/tmp/a b"),
                OsString::from("--"),
                OsString::from("'\"$(id)`;\n-x"),
            ])
            .build()
    }

    /// The line is only quoted references, led by a space, guarded on the
    /// program's variable and ending in `&& exit`, whatever the words hold;
    /// each word travels unchanged in its own variable, the program first.
    #[test]
    fn the_typed_line_is_only_variable_references_led_by_a_space() {
        let (env, line) = typed_line(&hostile());
        assert_eq!(
            line,
            " [ -n \"$MUSTER_OPEN_ARG_0\" ] && \"$MUSTER_OPEN_ARG_0\" \"$MUSTER_OPEN_ARG_1\" \
             \"$MUSTER_OPEN_ARG_2\" \"$MUSTER_OPEN_ARG_3\" \"$MUSTER_OPEN_ARG_4\" && exit"
        );
        assert_eq!(env, [
            (
                "MUSTER_OPEN_ARG_0".to_string(),
                OsString::from("/opt/my muster/muster")
            ),
            ("MUSTER_OPEN_ARG_1".to_string(), OsString::from("--project")),
            ("MUSTER_OPEN_ARG_2".to_string(), OsString::from("/tmp/a b")),
            ("MUSTER_OPEN_ARG_3".to_string(), OsString::from("--")),
            (
                "MUSTER_OPEN_ARG_4".to_string(),
                OsString::from("'\"$(id)`;\n-x")
            ),
        ]);
        assert_eq!(env[0].0, MUSTER_OPEN_PROGRAM_ENV);
    }
}

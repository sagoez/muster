use std::{env, ffi::OsString, process::Command};

/// What every variable herdr sets in its panes starts with.
const HERDR_ENV_PREFIX: &str = "HERDR_";
/// The variable naming the herdr pane a command runs in.
pub const HERDR_PANE_ENV: &str = "HERDR_PANE_ID";
/// The variable naming the herdr binary.
pub const HERDR_BIN_ENV: &str = "HERDR_BIN_PATH";

/// Strips from `command` every herdr variable this test's environment holds,
/// so a suite run inside a herdr pane never opens a pane in it.
pub fn without_herdr(command: &mut Command) -> &mut Command {
    without_herdr_of(command, env::vars_os())
}

/// Strips from `command` every herdr variable `environment` holds.
fn without_herdr_of(
    command: &mut Command,
    environment: impl IntoIterator<Item = (OsString, OsString)>,
) -> &mut Command {
    for (name, _) in environment {
        if name
            .as_encoded_bytes()
            .starts_with(HERDR_ENV_PREFIX.as_bytes())
        {
            command.env_remove(name);
        }
    }
    command
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    /// Every herdr variable of the environment is removed, and nothing else
    /// is touched.
    #[test]
    fn every_herdr_variable_is_removed_and_nothing_else() {
        let mut command = Command::new("true");
        let environment = [
            ("HERDR_PANE_ID", "w1:p1"),
            ("HERDR_BIN_PATH", "/usr/bin/herdr"),
            ("PATH", "/usr/bin"),
            ("MY_HERDR_NOTE", "kept"),
        ]
        .map(|(name, value)| (OsString::from(name), OsString::from(value)));

        without_herdr_of(&mut command, environment);

        let mut changed: Vec<(&OsStr, Option<&OsStr>)> = command.get_envs().collect();
        changed.sort();
        assert_eq!(changed, [
            (OsStr::new("HERDR_BIN_PATH"), None),
            (OsStr::new("HERDR_PANE_ID"), None),
        ]);
    }
}

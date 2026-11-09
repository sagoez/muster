//! The line a command ends with when there is something to run next.

use muster_domain::value::{ProcessName, ProjectName};

/// Opens every next-step line.
const RUN: &str = "Run";
/// The command that shows a process's log, before its name.
const LOGS_COMMAND: &str = "muster logs";
/// The flag naming the project a command acts on.
const PROJECT_FLAG: &str = "-p";
/// Ends the line pointing at a failed process's log.
const TO_SEE_WHY: &str = "to see why it failed.";
/// What to run when every process is stopped or never started.
pub const START_THEM: &str = "Run muster up to start them.";
/// What to run in a project with no processes.
pub const ADD_ONE: &str = "Run muster add <name> -- <command> to add one.";
/// What to run when no project is loaded.
pub const UP_IN_A_PROJECT: &str = "Run muster up in a project folder to start its processes.";

/// `Run muster logs <name> to see why it failed.`, naming `project` too
/// when given, each shell-quoted when it needs to be; `None` when a name no
/// shell word can carry.
pub fn see_why(name: &ProcessName, project: Option<&ProjectName>) -> Option<String> {
    let process = shlex::try_quote(name.as_ref()).ok()?;
    let project = match project {
        Some(project) => format!(
            " {PROJECT_FLAG} {}",
            shlex::try_quote(project.as_ref()).ok()?
        ),
        None => String::new(),
    };
    Some(format!(
        "{RUN} {LOGS_COMMAND} {process}{project} {TO_SEE_WHY}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name a shell would split is quoted, and the project is named only
    /// when asked.
    #[test]
    fn the_logs_step_quotes_what_it_names() {
        let name = |raw| ProcessName::try_new(raw).unwrap();
        assert_eq!(
            see_why(&name("infra"), None).as_deref(),
            Some("Run muster logs infra to see why it failed.")
        );
        assert_eq!(
            see_why(
                &name("dev server"),
                Some(&ProjectName::try_new("my shop").unwrap())
            )
            .as_deref(),
            Some("Run muster logs 'dev server' -p 'my shop' to see why it failed.")
        );
    }
}

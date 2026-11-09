//! The line a command ends with when there is something to run next, as a
//! command that works when pasted: each name shell-quoted, the project named
//! when the command needs it, and a name that reads as a flag passed after
//! `--`.

use muster_domain::{
    process::PatternKind,
    value::{OutputCursor, ProcessName, ProjectName},
};
use muster_protocol::service::WaitCondition;

/// Opens every next-step line.
const RUN: &str = "Run";
/// The command that shows a process's log.
const LOGS_COMMAND: &str = "muster logs";
/// The command that starts a process.
const START_COMMAND: &str = "muster start";
/// The command that starts the processes that autostart.
const UP_COMMAND: &str = "muster up";
/// The command that adds a process.
const ADD_COMMAND: &str = "muster add";
/// What that command takes.
const ADD_OPERANDS: &str = "<name> -- <command>";
/// The command that waits on a process.
const WAIT_COMMAND: &str = "muster wait";
/// The flag continuing a wait from a cursor.
const SINCE_FLAG: &str = "--since";
/// The flags of `muster wait` that name what it waits for.
const LOG_FLAG: &str = "--log";
const REGEX_FLAG: &str = "--regex";
const PORT_FLAG: &str = "--port";
const HTTP_FLAG: &str = "--http";
const STATUS_FLAG: &str = "--status";
const STATE_FLAG: &str = "--state";
/// The flag naming the project, before its value.
const PROJECT_FLAG: &str = "-p";
/// The flag naming the project, joined to a value that reads as a flag.
const PROJECT_FLAG_JOINED: &str = "--project=";
/// What starts a word the command line would read as a flag.
const FLAG_START: char = '-';
/// Ends the options, so a name that reads as a flag is taken as a name.
const END_OF_OPTIONS: &str = "--";
/// How each step ends.
const TO_SEE_WHY: &str = "to see why it failed.";
const TO_SEE_WHAT_IT_PRINTED: &str = "to see what it printed.";
const TO_SEE_WHY_NOT_READY: &str = "to see why it is not ready.";
const TO_START_IT: &str = "to start it.";
const TO_START_THEM: &str = "to start them.";
const TO_ADD_ONE: &str = "to add one.";
const TO_KEEP_WAITING: &str = "to keep waiting.";
/// What to run when no project is loaded.
pub const UP_IN_A_PROJECT: &str = "Run muster up in a project folder to start its processes.";

/// `Run muster logs <name> to see why it failed.`, naming `project` when
/// given; `None` for a name no shell word can carry.
pub fn see_why(name: &ProcessName, project: Option<&ProjectName>) -> Option<String> {
    on_process(LOGS_COMMAND, name, project, &[], TO_SEE_WHY)
}

/// `Run muster logs <name> to see what it printed.`, as [`see_why`] names
/// it, for a process that ended before what was waited for.
pub fn see_output(name: &ProcessName, project: Option<&ProjectName>) -> Option<String> {
    on_process(LOGS_COMMAND, name, project, &[], TO_SEE_WHAT_IT_PRINTED)
}

/// `Run muster logs <name> to see why it is not ready.`, as [`see_why`]
/// names it.
pub fn see_why_not_ready(name: &ProcessName, project: Option<&ProjectName>) -> Option<String> {
    on_process(LOGS_COMMAND, name, project, &[], TO_SEE_WHY_NOT_READY)
}

/// `Run muster start <name> to start it.`, as [`see_why`] names it.
pub fn start_it(name: &ProcessName, project: Option<&ProjectName>) -> Option<String> {
    on_process(START_COMMAND, name, project, &[], TO_START_IT)
}

/// `Run muster up to start them.`, naming `project` when given.
pub fn start_them(project: Option<&ProjectName>) -> Option<String> {
    Some(format!(
        "{RUN} {UP_COMMAND}{} {TO_START_THEM}",
        project_words(project)?
    ))
}

/// `Run muster add <name> -- <command> to add one.`, naming `project` when
/// given.
pub fn add_one(project: Option<&ProjectName>) -> Option<String> {
    Some(format!(
        "{RUN} {ADD_COMMAND}{} {ADD_OPERANDS} {TO_ADD_ONE}",
        project_words(project)?
    ))
}

/// `Run muster wait <name> --since <cursor> ... to keep waiting.`, waiting
/// again for `until` from `cursor`, naming `project` when given.
pub fn keep_waiting(
    name: &ProcessName,
    project: Option<&ProjectName>,
    until: &WaitCondition,
    cursor: OutputCursor,
) -> Option<String> {
    let mut flags = vec![SINCE_FLAG.to_string(), cursor.to_string()];
    match until {
        WaitCondition::Log(pattern) => {
            flags.push(LOG_FLAG.to_string());
            flags.push(shlex::try_quote(pattern.text().as_ref()).ok()?.into_owned());
            if *pattern.kind() == PatternKind::Regex {
                flags.push(REGEX_FLAG.to_string());
            }
        },
        WaitCondition::Port(port) => {
            flags.push(PORT_FLAG.to_string());
            flags.push(port.to_string());
        },
        WaitCondition::Http { url, status } => {
            flags.push(HTTP_FLAG.to_string());
            flags.push(shlex::try_quote(url.as_ref()).ok()?.into_owned());
            if let Some(status) = status {
                flags.push(STATUS_FLAG.to_string());
                flags.push(status.to_string());
            }
        },
        WaitCondition::State(goal) => {
            flags.push(STATE_FLAG.to_string());
            flags.push(goal.to_string());
        },
        WaitCondition::Ready => {},
    }
    on_process(WAIT_COMMAND, name, project, &flags, TO_KEEP_WAITING)
}

/// `Run <command> [-p <project>] <flags> <name> <why>`.
fn on_process(
    command: &str,
    name: &ProcessName,
    project: Option<&ProjectName>,
    flags: &[String],
    why: &str,
) -> Option<String> {
    let quoted = shlex::try_quote(name.as_ref()).ok()?;
    let name = if name.as_ref().starts_with(FLAG_START) {
        format!("{END_OF_OPTIONS} {quoted}")
    } else {
        quoted.into_owned()
    };
    let flags: String = flags.iter().map(|flag| format!(" {flag}")).collect();
    Some(format!(
        "{RUN} {command}{}{flags} {name} {why}",
        project_words(project)?
    ))
}

/// ` -p <project>` for `project`, joined as ` --project=<project>` when it
/// reads as a flag, or nothing without one; `None` for a name no shell word
/// can carry.
fn project_words(project: Option<&ProjectName>) -> Option<String> {
    let Some(project) = project else {
        return Some(String::new());
    };
    let quoted = shlex::try_quote(project.as_ref()).ok()?;
    Some(if project.as_ref().starts_with(FLAG_START) {
        format!(" {PROJECT_FLAG_JOINED}{quoted}")
    } else {
        format!(" {PROJECT_FLAG} {quoted}")
    })
}

#[cfg(test)]
mod tests {
    use muster_domain::value::LocalPort;

    use super::*;

    /// The process `raw`.
    fn name(raw: &str) -> ProcessName {
        ProcessName::try_new(raw).unwrap()
    }

    /// The project `raw`.
    fn project(raw: &str) -> ProjectName {
        ProjectName::try_new(raw).unwrap()
    }

    /// A name a shell would split is quoted, and the project is named only
    /// when asked.
    #[test]
    fn a_step_quotes_what_it_names() {
        assert_eq!(
            see_why(&name("infra"), None).as_deref(),
            Some("Run muster logs infra to see why it failed.")
        );
        assert_eq!(
            see_why(&name("dev server"), Some(&project("my shop"))).as_deref(),
            Some("Run muster logs -p 'my shop' 'dev server' to see why it failed.")
        );
        assert_eq!(
            start_it(&name("api"), None).as_deref(),
            Some("Run muster start api to start it.")
        );
        assert_eq!(
            add_one(Some(&project("blog"))).as_deref(),
            Some("Run muster add -p blog <name> -- <command> to add one.")
        );
        assert_eq!(
            start_them(None).as_deref(),
            Some("Run muster up to start them.")
        );
    }

    /// A name or project that reads as a flag is passed so the command line
    /// takes it as a value.
    #[test]
    fn a_name_that_reads_as_a_flag_stays_a_name() {
        assert_eq!(
            see_why(&name("--follow"), Some(&project("-x"))).as_deref(),
            Some("Run muster logs --project=-x -- --follow to see why it failed.")
        );
    }

    /// A wait that ran out of time is continued from its cursor, for what it
    /// waited for.
    #[test]
    fn keep_waiting_names_what_it_waited_for() {
        assert_eq!(
            keep_waiting(
                &name("api"),
                Some(&project("shop")),
                &WaitCondition::Port(LocalPort::try_new(3000).unwrap()),
                OutputCursor::new(120),
            )
            .as_deref(),
            Some("Run muster wait -p shop --since o:120 --port 3000 api to keep waiting.")
        );
    }
}

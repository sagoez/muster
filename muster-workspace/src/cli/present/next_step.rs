//! The line a command ends with when there is something to run next, as a
//! command that works when pasted: each name shell-quoted, the project named
//! when the command needs it, and a name that reads as a flag passed after
//! `--`.

use std::path::Path;

use muster_client::{process_operand, project_option_for};
use muster_domain::{
    process::PatternKind,
    project::Project,
    value::{OutputCursor, ProcessName},
};
use muster_protocol::service::WaitCondition;

use crate::cli::command_line::CommandName;

/// Opens every next-step line.
const RUN: &str = "Run";
/// What `muster add` takes.
const ADD_OPERANDS: &str = "<name> -- <command>";
/// The flag continuing a wait from a cursor.
const SINCE_FLAG: &str = "--since";
/// The flags of `muster wait` that name what it waits for.
const LOG_FLAG: &str = "--log";
const REGEX_FLAG: &str = "--regex";
const PORT_FLAG: &str = "--port";
const HTTP_FLAG: &str = "--http";
const STATUS_FLAG: &str = "--status";
const STATE_FLAG: &str = "--state";
/// Joins a long flag to its value, so a value that reads as a flag is
/// still taken as the value.
const FLAG_VALUE_JOIN: &str = "=";
/// Ends the step after a run that never launched.
const TO_SEE_WHY_NOT_LAUNCHED: &str = "to see why it did not start.";
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
pub fn see_why(
    name: &ProcessName,
    project: Option<&Project>,
    home: Option<&Path>,
) -> Option<String> {
    on_process(CommandName::Logs, name, project, home, &[], TO_SEE_WHY)
}

/// `Run muster logs <name> to see what it printed.`, as [`see_why`] names
/// it, for a process that ended before what was waited for.
pub fn see_output(
    name: &ProcessName,
    project: Option<&Project>,
    home: Option<&Path>,
) -> Option<String> {
    on_process(
        CommandName::Logs,
        name,
        project,
        home,
        &[],
        TO_SEE_WHAT_IT_PRINTED,
    )
}

/// `Run muster logs <name> to see why it is not ready.`, as [`see_why`]
/// names it.
pub fn see_why_not_ready(
    name: &ProcessName,
    project: Option<&Project>,
    home: Option<&Path>,
) -> Option<String> {
    on_process(
        CommandName::Logs,
        name,
        project,
        home,
        &[],
        TO_SEE_WHY_NOT_READY,
    )
}

/// `Run muster start <name> to start it.`, as [`see_why`] names it.
pub fn start_it(
    name: &ProcessName,
    project: Option<&Project>,
    home: Option<&Path>,
) -> Option<String> {
    on_process(CommandName::Start, name, project, home, &[], TO_START_IT)
}

/// `Run muster notices to see why it did not start.`, naming `project` when
/// given, for a run that never launched and so logged nothing.
pub fn see_why_not_launched(project: Option<&Project>, home: Option<&Path>) -> Option<String> {
    Some(format!(
        "{RUN} {}{} {TO_SEE_WHY_NOT_LAUNCHED}",
        CommandName::Notices.invocation(),
        project_words(project, home)?
    ))
}

/// `Run muster up to start them.`, naming `project` when given.
pub fn start_them(project: Option<&Project>, home: Option<&Path>) -> Option<String> {
    Some(format!(
        "{RUN} {}{} {TO_START_THEM}",
        CommandName::Up.invocation(),
        project_words(project, home)?
    ))
}

/// `Run muster add <name> -- <command> to add one.`, naming `project` when
/// given.
pub fn add_one(project: Option<&Project>, home: Option<&Path>) -> Option<String> {
    Some(format!(
        "{RUN} {}{} {ADD_OPERANDS} {TO_ADD_ONE}",
        CommandName::Add.invocation(),
        project_words(project, home)?
    ))
}

/// `Run muster wait <name> --since <cursor> ... to keep waiting.`, waiting
/// again for `until` from `cursor`, naming `project` when given.
pub fn keep_waiting(
    name: &ProcessName,
    project: Option<&Project>,
    home: Option<&Path>,
    until: &WaitCondition,
    cursor: OutputCursor,
) -> Option<String> {
    let mut flags = vec![SINCE_FLAG.to_string(), cursor.to_string()];
    match until {
        WaitCondition::Log(pattern) => {
            let text = shlex::try_quote(pattern.text().as_ref()).ok()?;
            flags.push(format!("{LOG_FLAG}{FLAG_VALUE_JOIN}{text}"));
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
    on_process(
        CommandName::Wait,
        name,
        project,
        home,
        &flags,
        TO_KEEP_WAITING,
    )
}

/// `Run muster <command> [-p <project>] <flags> <name> <why>`.
fn on_process(
    command: CommandName,
    name: &ProcessName,
    project: Option<&Project>,
    home: Option<&Path>,
    flags: &[String],
    why: &str,
) -> Option<String> {
    let name = process_operand(name)?;
    let flags: String = flags.iter().map(|flag| format!(" {flag}")).collect();
    Some(format!(
        "{RUN} {}{}{flags} {name} {why}",
        command.invocation(),
        project_words(project, home)?
    ))
}

/// ` -p <project>` for `project`, joined as ` --project=<project>` when it
/// reads as a flag, and naming its folder written from `~` under `home`
/// when its name reads as a folder, which `-p` would look for wherever the
/// step is pasted; nothing without one, and `None` for a name or folder no
/// shell word can carry.
fn project_words(project: Option<&Project>, home: Option<&Path>) -> Option<String> {
    project.map_or_else(
        || Some(String::new()),
        |project| Some(format!(" {}", project_option_for(project, home)?)),
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use muster_domain::{
        process::{LogPattern, PatternText},
        value::{LocalPort, ProjectName, ProjectRoot},
    };

    use super::*;

    /// The home folder the tests' projects live under.
    const HOME: &str = "/home/me";

    /// The process `raw`.
    fn name(raw: &str) -> ProcessName {
        ProcessName::try_new(raw).unwrap()
    }

    /// The project named `raw`, in a folder of that name under [`HOME`].
    fn project(raw: &str) -> Project {
        Project::builder()
            .root(ProjectRoot::try_new(PathBuf::from(HOME).join("code").join(raw)).unwrap())
            .name(ProjectName::try_new(raw).unwrap())
            .build()
    }

    /// A project whose name reads as a folder is named by its folder,
    /// written from `~`, so pasted where a folder of that name sits the
    /// step still reaches it.
    #[test]
    fn a_folder_like_project_is_named_by_its_folder() {
        assert_eq!(
            start_it(
                &name("lint"),
                Some(&project(".github")),
                Some(Path::new(HOME))
            )
            .as_deref(),
            Some("Run muster start -p '~/code/.github' lint to start it.")
        );
        assert_eq!(
            start_them(Some(&project("~shop")), None).as_deref(),
            Some("Run muster up -p '/home/me/code/~shop' to start them.")
        );
    }

    /// A name a shell would split is quoted, and the project is named only
    /// when asked.
    #[test]
    fn a_step_quotes_what_it_names() {
        assert_eq!(
            see_why(&name("infra"), None, None).as_deref(),
            Some("Run muster logs infra to see why it failed.")
        );
        assert_eq!(
            see_why(&name("dev server"), Some(&project("my shop")), None).as_deref(),
            Some("Run muster logs -p 'my shop' 'dev server' to see why it failed.")
        );
        assert_eq!(
            start_it(&name("api"), None, None).as_deref(),
            Some("Run muster start api to start it.")
        );
        assert_eq!(
            add_one(Some(&project("blog")), None).as_deref(),
            Some("Run muster add -p blog <name> -- <command> to add one.")
        );
        assert_eq!(
            start_them(None, None).as_deref(),
            Some("Run muster up to start them.")
        );
    }

    /// A name or project that reads as a flag is passed so the command line
    /// takes it as a value.
    #[test]
    fn a_name_that_reads_as_a_flag_stays_a_name() {
        assert_eq!(
            see_why(&name("--follow"), Some(&project("-x")), None).as_deref(),
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
                None,
                &WaitCondition::Port(LocalPort::try_new(3000).unwrap()),
                OutputCursor::new(120),
            )
            .as_deref(),
            Some("Run muster wait -p shop --since o:120 --port 3000 api to keep waiting.")
        );
        let dashed = LogPattern::builder()
            .text(PatternText::try_new("-nope").unwrap())
            .kind(PatternKind::Substring)
            .build();
        assert_eq!(
            keep_waiting(
                &name("web"),
                None,
                None,
                &WaitCondition::Log(dashed),
                OutputCursor::new(18),
            )
            .as_deref(),
            Some("Run muster wait --since o:18 --log=-nope web to keep waiting."),
            "a pattern that reads as a flag is joined to its flag"
        );
    }
}

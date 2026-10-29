//! `muster set`: changes to one process's stored definition. A running
//! process keeps running and takes the change at its next start.

use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{
    args::SetArgs,
    command_words::command_string,
    error::CliError,
    invocation::{Invocation, stack_editor},
    output::project_title,
    process_flags::env_name,
    report::{Report, Row, RowKind},
    working_dir::stored_working_dir,
};
use crate::{
    adapter::client::{
        DefinitionView, ProjectIdentityView,
        process::{resolve_among, runs_now},
    },
    domain::{
        config::{ConfigError, FieldEdit, ProcessChange},
        port::StackStore,
        value::Description,
    },
};

/// The command word of `muster set`.
const SET_WORD: &str = "set";
/// Opens the line naming the changed process.
const UPDATED: &str = "updated";
/// Between a name and what changed.
const GAP: &str = "  ";
/// Between the changes listed.
const CHANGE_SEPARATOR: &str = ", ";
/// Names each kind of change in the summary.
const COMMAND_CHANGED: &str = "command";
const CWD_CHANGED: &str = "cwd";
const CWD_CLEARED: &str = "cwd cleared";
const DESCRIPTION_CHANGED: &str = "description";
const DESCRIPTION_CLEARED: &str = "description cleared";
const ENV_SET: &str = "env";
const ENV_UNSET: &str = "unset env";
const AUTOSTART_ON: &str = "autostart on";
const AUTOSTART_OFF: &str = "autostart off";
const RESTART_CHANGED: &str = "restart";
const STOP_SIGNAL_CHANGED: &str = "stop-signal";
const STOP_GRACE_CHANGED: &str = "stop-grace";
/// Follows a running process's name in the restart hint.
const IS_RUNNING: &str = "is running;";
/// The command that applies a change to a running process.
const RESTART_COMMAND: &str = "muster restart";
/// Follows that command in the hint.
const APPLIES_IT: &str = "applies the change";

/// What `muster set` did, as `--json` shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct SetView {
    /// The process's project.
    #[getset(get = "pub")]
    project: ProjectIdentityView,
    /// The process as stored now.
    #[getset(get = "pub")]
    process: DefinitionView,
    /// Whether a running service reports it running, so the change waits
    /// for its next start.
    #[getset(get_copy = "pub")]
    running: bool,
    /// What changed, in words.
    #[serde(skip)]
    #[getset(get = "pub")]
    changes: Vec<String>,
}

/// Applies the changes `args` asks for to one process of the invocation's
/// project. A running service is asked whether the process runs, never
/// started for it; one that cannot be asked counts as not running, since the
/// change is stored by then.
///
/// # Errors
/// Returns [`CliError::NothingToChange`] when no change was given, the
/// [`CliError`] of an unknown process or an unusable value, or a stored-state
/// failure.
pub fn set(invocation: &Invocation<'_>, args: SetArgs) -> Result<SetView, CliError> {
    if asks_nothing(&args) {
        return Err(CliError::NothingToChange);
    }
    let store = invocation.store()?;
    let project = invocation.project(&store)?;
    let root = project.root();
    let words = (!args.words.is_empty())
        .then(|| command_string(&args.words))
        .transpose()?;
    let working_dir = match (args.cwd, args.no_cwd) {
        (Some(dir), _) => match stored_working_dir(root, &invocation.in_cwd(&dir)?)? {
            Some(dir) => FieldEdit::Set(dir),
            None => FieldEdit::Clear,
        },
        (None, true) => FieldEdit::Clear,
        (None, false) => FieldEdit::Keep,
    };
    let description = match (args.description, args.no_description) {
        (Some(text), _) => {
            FieldEdit::Set(Description::try_new(text).map_err(|_| CliError::EmptyDescription)?)
        },
        (None, true) => FieldEdit::Clear,
        (None, false) => FieldEdit::Keep,
    };
    let change = ProcessChange::builder()
        .command(args.command.or(words))
        .working_dir(working_dir)
        .description(description)
        .set_env(args.flags.environment()?)
        .unset_env(
            args.unset_env
                .iter()
                .map(|raw| env_name(raw))
                .collect::<Result<_, _>>()?,
        )
        .autostart(args.flags.autostart())
        .restart(args.flags.restart)
        .stop_signal(args.flags.stop_signal)
        .stop_grace(args.flags.stop_grace)
        .build();
    let stack = store
        .stack(root)?
        .ok_or_else(|| ConfigError::UnknownProject {
            root: root.as_ref().clone(),
        })?;
    let name = resolve_among(stack.names(), &project, &args.name)?;
    let spec = stack_editor(&store).change(root, &name, &change)?;
    let running = runs_now(root, spec.name());
    Ok(SetView::builder()
        .project(ProjectIdentityView::from(&project))
        .process(DefinitionView::from(&spec))
        .running(running)
        .changes(change_words(&change))
        .build())
}

/// Whether `args` asks for no change at all, which is refused before any
/// state is read.
fn asks_nothing(args: &SetArgs) -> bool {
    args.command.is_none()
        && args.words.is_empty()
        && args.cwd.is_none()
        && !args.no_cwd
        && args.description.is_none()
        && !args.no_description
        && args.unset_env.is_empty()
        && args.flags.is_empty()
}

/// What `change` changes, in words: `restart on-failure`, `env PORT`.
fn change_words(change: &ProcessChange) -> Vec<String> {
    let mut words = Vec::new();
    if change.command().is_some() {
        words.push(COMMAND_CHANGED.to_string());
    }
    match change.working_dir() {
        FieldEdit::Set(_) => words.push(CWD_CHANGED.to_string()),
        FieldEdit::Clear => words.push(CWD_CLEARED.to_string()),
        FieldEdit::Keep => {},
    }
    match change.description() {
        FieldEdit::Set(_) => words.push(DESCRIPTION_CHANGED.to_string()),
        FieldEdit::Clear => words.push(DESCRIPTION_CLEARED.to_string()),
        FieldEdit::Keep => {},
    }
    words.extend(
        change
            .set_env()
            .iter()
            .map(|(name, _)| format!("{ENV_SET} {name}")),
    );
    words.extend(
        change
            .unset_env()
            .iter()
            .map(|name| format!("{ENV_UNSET} {name}")),
    );
    if let Some(autostart) = change.autostart() {
        words.push(
            if *autostart {
                AUTOSTART_ON
            } else {
                AUTOSTART_OFF
            }
            .to_string(),
        );
    }
    if let Some(restart) = change.restart() {
        words.push(format!("{RESTART_CHANGED} {restart}"));
    }
    if let Some(signal) = change.stop_signal() {
        words.push(format!("{STOP_SIGNAL_CHANGED} {signal}"));
    }
    if let Some(grace) = change.stop_grace() {
        words.push(format!("{STOP_GRACE_CHANGED} {grace}"));
    }
    words
}

/// The human report of `view`: what changed, and how a running process
/// takes the change.
pub fn set_rows(view: &SetView) -> Report {
    let name = view.process().name();
    let mut rows = vec![Row::unlabeled(
        RowKind::Ok,
        format!(
            "{UPDATED} {name}{GAP}{}",
            view.changes().join(CHANGE_SEPARATOR)
        ),
    )];
    if view.running() {
        rows.push(Row::unlabeled(
            RowKind::Dim,
            format!(
                "{name} {IS_RUNNING} `{RESTART_COMMAND} {}` {APPLIES_IT}",
                name.shell_quoted()
            ),
        ));
    }
    Report::new(project_title(SET_WORD, view.project().name()), rows)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::{
        process::RestartPolicy,
        value::{EnvName, EnvValue, PositiveDuration},
    };

    /// Each kind of change is named in the summary, variables by name only.
    #[test]
    fn a_change_is_summarised_in_words() {
        let change = ProcessChange::builder()
            .working_dir(FieldEdit::Clear)
            .set_env(vec![(
                EnvName::try_new("PORT").unwrap(),
                EnvValue::try_new("secret").unwrap(),
            )])
            .restart(Some(RestartPolicy::OnFailure))
            .stop_grace(Some(
                PositiveDuration::try_new(Duration::from_secs(10)).unwrap(),
            ))
            .build();

        assert_eq!(change_words(&change), vec![
            "cwd cleared",
            "env PORT",
            "restart on-failure",
            "stop-grace 10s"
        ]);
    }
}

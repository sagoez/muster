use std::{io, path::PathBuf, process::ExitStatus, time::Duration};

use humantime::format_duration;
use thiserror::Error;

use super::reply::ErrorReply;
use crate::{adapter::sanitize::printable, domain::terminal_workspace::WorkspaceError};

/// The name muster gives herdr in what it reports.
pub(super) const WORKSPACE_NAME: &str = "herdr";
/// The most characters of any one text from herdr muster passes on.
const MAX_HERDR_TEXT_CHARS: usize = 512;
/// Ends a text from herdr cut at [`MAX_HERDR_TEXT_CHARS`].
const CUT_MARK: &str = "...";
/// Stands in for a refusal herdr printed no words for.
const NO_OUTPUT: &str = "no output";
/// Leads what herdr was asked when it could not be reached.
const ASKED_TO: &str = "asked to";

/// Why one run of the herdr CLI did not answer.
#[derive(Debug, Error)]
pub(super) enum HerdrFailure {
    /// The binary could not be started.
    #[error("could not start {}: {source}", program.display())]
    Spawn {
        /// The binary muster tried to start.
        program: PathBuf,
        /// Why it would not start.
        source: io::Error,
    },
    /// Waiting on the started binary failed.
    #[error("lost track of it: {0}")]
    Wait(io::Error),
    /// It did not finish within the given time, and was killed and reaped.
    #[error("it did not answer within {}, so it was stopped", format_duration(*.0))]
    TimedOut(Duration),
    /// What it printed could not be read: too large, or not UTF-8.
    #[error("{0}")]
    Unreadable(io::Error),
    /// It exited unsuccessfully, with what it printed on stderr.
    #[error("it ended with {status}: {stderr}")]
    Exited {
        /// How it exited.
        status: ExitStatus,
        /// What it printed on stderr.
        stderr: String,
    },
}

impl HerdrFailure {
    /// This failure as the port reports it, naming `action`, what herdr was
    /// asked to do; `unanswered`, when given, is appended to a timeout to say
    /// what herdr may have done before it was stopped.
    pub(super) fn into_workspace_error(
        self,
        action: &str,
        unanswered: Option<&str>,
    ) -> WorkspaceError {
        let unreachable = |reason: String| WorkspaceError::Unreachable {
            workspace: WORKSPACE_NAME.to_string(),
            reason: herdr_text(&format!("{ASKED_TO} {action}, {reason}")),
        };
        match self {
            Self::Spawn { .. } | Self::Wait(_) => unreachable(self.to_string()),
            Self::TimedOut(_) => unreachable(format!("{self}{}", unanswered.unwrap_or_default())),
            Self::Unreadable(_) => WorkspaceError::Unreadable {
                workspace: WORKSPACE_NAME.to_string(),
                action: herdr_text(action),
                reason: herdr_text(&self.to_string()),
            },
            Self::Exited { status, stderr } => {
                let (code, message) = match serde_json::from_str::<ErrorReply>(&stderr) {
                    Ok(reply) => reply.into_parts(),
                    Err(_) => (status.to_string(), stderr),
                };
                let message = if message.trim().is_empty() {
                    NO_OUTPUT.to_string()
                } else {
                    message
                };
                WorkspaceError::Refused {
                    workspace: WORKSPACE_NAME.to_string(),
                    action: herdr_text(action),
                    code: herdr_text(&code),
                    message: herdr_text(&message),
                }
            },
        }
    }
}

/// `text` from herdr made safe to print and cut to
/// [`MAX_HERDR_TEXT_CHARS`], so a hostile or broken herdr can neither steer
/// the user's terminal nor flood it.
pub(super) fn herdr_text(text: &str) -> String {
    let safe = printable(text.trim());
    if safe.chars().count() <= MAX_HERDR_TEXT_CHARS {
        return safe;
    }
    let mut cut: String = safe.chars().take(MAX_HERDR_TEXT_CHARS).collect();
    cut.push_str(CUT_MARK);
    cut
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::*;

    /// Control characters are shown as text and a long text is cut.
    #[test]
    fn herdr_text_is_printable_and_bounded() {
        assert_eq!(herdr_text(" a\u{1b}[2Jb\n"), "a\\u{1b}[2Jb");
        let long = herdr_text(&"x".repeat(MAX_HERDR_TEXT_CHARS * 2));
        assert_eq!(long.chars().count(), MAX_HERDR_TEXT_CHARS + CUT_MARK.len());
        assert!(long.ends_with(CUT_MARK));
    }

    /// A binary that would not start is unreachable, naming what was asked
    /// and the path tried, made safe to print.
    #[test]
    fn a_spawn_failure_names_the_action_and_the_binary() {
        let failure = HerdrFailure::Spawn {
            program: PathBuf::from("/opt/herdr\u{1b}[2J/herdr"),
            source: io::Error::from(ErrorKind::NotFound),
        };
        let WorkspaceError::Unreachable { workspace, reason } =
            failure.into_workspace_error("split pane w1:p1", None)
        else {
            panic!("not unreachable");
        };
        assert_eq!(workspace, WORKSPACE_NAME);
        assert_eq!(
            reason,
            "asked to split pane w1:p1, could not start /opt/herdr\\u{1b}[2J/herdr: entity not found"
        );
    }

    /// A timeout appends what herdr may have done before it was stopped.
    #[test]
    fn a_timeout_appends_what_may_be_left() {
        let error = HerdrFailure::TimedOut(Duration::from_secs(5))
            .into_workspace_error("split pane w1:p1", Some("; a pane may have opened"));
        assert_eq!(
            error.to_string(),
            "cannot reach herdr: asked to split pane w1:p1, it did not answer within 5s, so \
             it was stopped; a pane may have opened"
        );
    }
}

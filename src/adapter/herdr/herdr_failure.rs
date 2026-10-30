use std::{io, process::ExitStatus, time::Duration};

use humantime::format_duration;

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
/// Opens why herdr could not be started.
const SPAWN_FAILED: &str = "could not start it to";
/// Opens why a started herdr was lost track of.
const WAIT_FAILED: &str = "lost track of it while asking it to";
/// Opens why herdr was stopped for not answering.
const TIMED_OUT: &str = "it did not answer when asked to";
/// Joins the time herdr was given to the action it was asked.
const WITHIN: &str = "within";
/// Says what became of a herdr that did not answer.
const STOPPED: &str = "so it was stopped";

/// Why one run of the herdr CLI did not answer.
#[derive(Debug)]
pub(super) enum HerdrFailure {
    /// The binary could not be started.
    Spawn(io::Error),
    /// Waiting on the started binary failed.
    Wait(io::Error),
    /// It did not finish within the given time, and was killed and reaped.
    TimedOut(Duration),
    /// What it printed could not be read: too large, or not UTF-8.
    Unreadable(io::Error),
    /// It exited unsuccessfully, with what it printed on stderr.
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
            reason: herdr_text(&reason),
        };
        match self {
            Self::Spawn(error) => unreachable(format!("{SPAWN_FAILED} {action}: {error}")),
            Self::Wait(error) => unreachable(format!("{WAIT_FAILED} {action}: {error}")),
            Self::TimedOut(timeout) => {
                let reason = format!(
                    "{TIMED_OUT} {action} {WITHIN} {}, {STOPPED}{}",
                    format_duration(timeout),
                    unanswered.unwrap_or_default()
                );
                unreachable(reason)
            },
            Self::Unreadable(error) => WorkspaceError::Unreadable {
                workspace: WORKSPACE_NAME.to_string(),
                action: herdr_text(action),
                reason: herdr_text(&error.to_string()),
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
    use super::*;

    /// Control characters are shown as text and a long text is cut.
    #[test]
    fn herdr_text_is_printable_and_bounded() {
        assert_eq!(herdr_text(" a\u{1b}[2Jb\n"), "a\\u{1b}[2Jb");
        let long = herdr_text(&"x".repeat(MAX_HERDR_TEXT_CHARS * 2));
        assert_eq!(long.chars().count(), MAX_HERDR_TEXT_CHARS + CUT_MARK.len());
        assert!(long.ends_with(CUT_MARK));
    }
}

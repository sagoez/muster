//! `muster wait` and `--wait`: waiting on the service for a process's output,
//! port, URL, state or readiness, without the bound an agent's call has.

use std::time::{Duration, Instant};

use muster_client::{ClientError, ElapsedSeconds};
use muster_domain::{
    process::PatternKind,
    value::{OutputCursor, ProjectName},
};
use muster_protocol::{
    holder::LineMatcher,
    service::{WaitCondition, WaitOutcome, WaitSpec, WaitStatus},
};
use strum::EnumMessage;

use super::{
    error::CliError,
    health::Health,
    next_step::{see_why, start_it},
    report::Report,
    row::Row,
    row_kind::RowKind,
    wait_args::WaitArgs,
};

/// Longest one wait request asks the service for; a longer wait is a series
/// of them, each continuing from where the last one stopped.
const WAIT_CHUNK: Duration = Duration::from_secs(30);
/// Title of a wait's report.
const WAIT_TITLE: &str = "muster wait";
/// Opens the line naming where the wait's output ends.
const CURSOR_WORD: &str = "cursor";
/// Opens the step after a wait that ran out of time, before its cursor.
const RUN_AGAIN: &str = "Run the wait again with --since";
/// Ends that step.
const TO_KEEP_WAITING: &str = "to keep waiting.";

/// What `args` asks to wait for, or `None` for the process's default.
///
/// # Errors
/// Returns [`CliError::InvalidPattern`] for a blank or overlong `--log`, or
/// a `--regex` that does not compile.
pub fn condition(args: &WaitArgs) -> Result<Option<WaitCondition>, CliError> {
    if let Some(log) = args.log() {
        return Ok(Some(WaitCondition::Log(LineMatcher::checked(
            log,
            PatternKind::of_regex(args.regex()),
        )?)));
    }
    Ok(args
        .port()
        .map(WaitCondition::Port)
        .or_else(|| {
            args.http().clone().map(|url| WaitCondition::Http {
                url,
                status: *args.status(),
            })
        })
        .or_else(|| args.state().map(WaitCondition::State)))
}

/// Waits for `until` (the process's default when `None`) from `since`, at
/// most `limit` or without end. `first` sends the first request, which may
/// carry an action; later requests continue from the cursor the last one
/// returned, until the wait ends or the limit passes.
///
/// # Errors
/// Returns the [`ClientError`] of a request the service refused or could
/// not answer.
pub fn wait_through(
    until: Option<WaitCondition>,
    since: Option<OutputCursor>,
    limit: Option<Duration>,
    first: impl FnOnce(&WaitSpec) -> Result<WaitOutcome, ClientError>,
    mut next: impl FnMut(&WaitSpec) -> Result<WaitOutcome, ClientError>,
) -> Result<WaitOutcome, ClientError> {
    let deadline = limit.map(|limit| Instant::now() + limit);
    let chunk = |since: Option<OutputCursor>, until: Option<WaitCondition>| {
        let left = deadline.map_or(WAIT_CHUNK, |deadline| {
            deadline.saturating_duration_since(Instant::now())
        });
        WaitSpec::builder()
            .until(until)
            .since(since)
            .timeout(left.min(WAIT_CHUNK))
            .build()
    };
    let mut outcome = first(&chunk(since, until))?;
    let mut waited = outcome.elapsed();
    while outcome.status() == WaitStatus::Waiting
        && deadline.is_none_or(|deadline| Instant::now() < deadline)
    {
        outcome = next(&chunk(
            Some(*outcome.cursor()),
            Some(outcome.until().clone()),
        ))?;
        waited = waited.saturating_add(outcome.elapsed());
    }
    Ok(outcome.with_elapsed(waited))
}

/// How a wait ended, for a person: a `◇` match, a `▲` wait that ran out of
/// time or a `■` failure, what was waited for and for how long, with the
/// matched line or the run's last lines dimmed under it, the cursor, and
/// what to run next, naming `project` when given.
pub fn wait_rows(outcome: &WaitOutcome, project: Option<&ProjectName>) -> Report {
    let elapsed = ElapsedSeconds::from(outcome.elapsed());
    let status = outcome.status();
    let ended = match status {
        WaitStatus::Crashed => Health::Failed.to_string(),
        status => status.get_message().unwrap_or_default().to_string(),
    };
    let kind = match status {
        WaitStatus::Matched => RowKind::Ok,
        WaitStatus::Waiting => RowKind::Hint,
        WaitStatus::Exited
        | WaitStatus::Crashed
        | WaitStatus::Stopped
        | WaitStatus::NotRunning
        | WaitStatus::ReadyTimeout => RowKind::Fail,
    };
    let row = Row::labeled(
        kind,
        outcome.process().to_string(),
        format!("{ended} waiting for {} after {elapsed}", outcome.until()),
    );
    let row = outcome
        .matched()
        .iter()
        .map(|matched| matched.line().clone())
        .chain(outcome.tail().iter().cloned())
        .fold(row, Row::with_note);
    let cursor = outcome.cursor();
    let next_step = match status {
        WaitStatus::Matched | WaitStatus::Stopped => None,
        WaitStatus::Waiting => Some(format!("{RUN_AGAIN} {cursor} {TO_KEEP_WAITING}")),
        WaitStatus::NotRunning => start_it(outcome.process(), project),
        WaitStatus::Exited | WaitStatus::Crashed | WaitStatus::ReadyTimeout => {
            see_why(outcome.process(), project)
        },
    };
    Report::new(WAIT_TITLE, vec![
        row,
        Row::unlabeled(RowKind::Plain, format!("{CURSOR_WORD} {cursor}")),
    ])
    .with_next_step(next_step)
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, thread};

    use muster_domain::value::ProcessName;
    use muster_protocol::service::{MatchedLine, StateGoal};

    use super::*;
    use crate::cli::report::plain;

    /// A wait that ended `status` with its output at `cursor`.
    fn outcome(status: WaitStatus, cursor: u64) -> WaitOutcome {
        WaitOutcome::builder()
            .process(ProcessName::try_new("api").unwrap())
            .status(status)
            .until(WaitCondition::State(StateGoal::Running))
            .cursor(OutputCursor::new(cursor))
            .elapsed(Duration::from_millis(2140))
            .build()
    }

    #[test]
    fn a_wait_continues_from_each_cursor_until_it_ends() {
        let sent = Cell::new(Vec::new());
        let calls = Cell::new(0u64);

        let ended = wait_through(
            None,
            None,
            None,
            |spec| {
                assert_eq!(*spec.since(), None);
                Ok(outcome(WaitStatus::Waiting, 10))
            },
            |spec| {
                let mut seen = sent.take();
                seen.push(*spec.since());
                sent.set(seen);
                calls.set(calls.get() + 1);
                let status = if calls.get() < 2 {
                    WaitStatus::Waiting
                } else {
                    WaitStatus::Matched
                };
                Ok(outcome(status, 10 + calls.get()))
            },
        )
        .unwrap();

        assert_eq!(ended.status(), WaitStatus::Matched);
        assert_eq!(sent.take(), [
            Some(OutputCursor::new(10)),
            Some(OutputCursor::new(11))
        ]);
        assert_eq!(
            ended.elapsed(),
            Duration::from_millis(3 * 2140),
            "elapsed counts every request"
        );
    }

    #[test]
    fn a_limit_ends_the_wait_as_waiting() {
        let ended = wait_through(
            None,
            None,
            Some(Duration::from_millis(1)),
            |spec| {
                assert!(*spec.timeout() <= Duration::from_millis(1));
                thread::sleep(Duration::from_millis(5));
                Ok(outcome(WaitStatus::Waiting, 0))
            },
            |_| panic!("the limit passed"),
        )
        .unwrap();

        assert_eq!(ended.status(), WaitStatus::Waiting);
    }

    #[test]
    fn a_failed_wait_shows_the_run_s_last_lines() {
        let crashed = WaitOutcome::builder()
            .process(ProcessName::try_new("api").unwrap())
            .status(WaitStatus::Crashed)
            .until(WaitCondition::Ready)
            .cursor(OutputCursor::new(90))
            .elapsed(Duration::from_millis(1200))
            .tail(vec!["Error: boom".to_string(), "    at main".to_string()])
            .build();
        let matched = WaitOutcome::builder()
            .process(ProcessName::try_new("api").unwrap())
            .status(WaitStatus::Matched)
            .until(WaitCondition::State(StateGoal::Running))
            .matched(Some(
                MatchedLine::builder()
                    .line("Listening on :3000".to_string())
                    .at(OutputCursor::new(4))
                    .build(),
            ))
            .cursor(OutputCursor::new(30))
            .elapsed(Duration::from_millis(5))
            .build();
        let timed_out = outcome(WaitStatus::Waiting, 120);
        let shop = ProjectName::try_new("shop").unwrap();

        insta::assert_snapshot!(format!(
            "{}\n---\n{}\n---\n{}",
            plain(&wait_rows(&crashed, None)),
            plain(&wait_rows(&matched, None)),
            plain(&wait_rows(&timed_out, Some(&shop)))
        ));
        assert!(
            plain(&wait_rows(&crashed, Some(&shop)))
                .ends_with("Run muster logs api -p shop to see why it failed.")
        );
        assert_eq!(
            plain(&wait_rows(&outcome(WaitStatus::NotRunning, 0), None))
                .lines()
                .last(),
            Some("Run muster start api to start it.")
        );
    }
}

use std::{ffi::OsString, path::PathBuf, time::Duration};

use serde::de::DeserializeOwned;

use super::{
    herdr_call::call,
    herdr_failure::{HerdrFailure, WORKSPACE_NAME, herdr_text},
    layout_reply::LayoutReply,
    pane_reply::PaneReply,
    typed_line::typed_line,
};
use crate::domain::{
    coordination::HERDR_PANE_ENV,
    port::TerminalWorkspace,
    terminal_workspace::{
        OpenedPane, PaneFocus, PaneRequest, SplitDirection, WorkspaceError, WorkspaceProbe,
    },
    value::{Cols, Rows, WorkspacePaneId},
};

/// How long one herdr call may take before it is killed.
pub const HERDR_CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// The herdr binary, found on `PATH`, when the environment names none.
const HERDR_PROGRAM: &str = "herdr";
/// The variable herdr sets in every pane to its own binary.
const HERDR_BIN_ENV: &str = "HERDR_BIN_PATH";
/// herdr's command group for panes.
const PANE: &str = "pane";
/// Reads a tab's layout.
const LAYOUT: &str = "layout";
/// Opens a pane beside another.
const SPLIT: &str = "split";
/// Types a line and Enter into a pane.
const RUN: &str = "run";
/// Closes a pane.
const CLOSE: &str = "close";
/// Describes a pane.
const GET: &str = "get";
/// Names the pane whose layout is read.
const PANE_FLAG: &str = "--pane";
/// Names the side a split opens on.
const DIRECTION_FLAG: &str = "--direction";
/// Names the folder a split's shell starts in.
const CWD_FLAG: &str = "--cwd";
/// Sets a variable for a split's shell.
const ENV_FLAG: &str = "--env";
/// Joins a variable's name to its value in [`ENV_FLAG`]'s argument.
const ENV_JOIN: &str = "=";
/// Focuses the new pane.
const FOCUS_FLAG: &str = "--focus";
/// Leaves the focus where it is.
const NO_FOCUS_FLAG: &str = "--no-focus";
/// What reading the layout is called in an error, before the pane.
const LAYOUT_ACTION: &str = "read the layout of pane";
/// What splitting is called in an error, before the pane.
const SPLIT_ACTION: &str = "split pane";
/// What typing the line is called in an error, before the pane.
const RUN_ACTION: &str = "run muster in pane";
/// What closing is called in an error, before the pane.
const CLOSE_ACTION: &str = "close pane";
/// What describing is called in an error, before the pane.
const GET_ACTION: &str = "describe pane";
/// Appended to a split that timed out.
const SPLIT_MAY_HAVE_OPENED: &str = "; a pane may have opened that muster never learned of";
/// Why a layout that does not list the caller cannot be used.
const CALLER_NOT_IN_LAYOUT: &str = "the layout does not list the pane";
/// Why a pane size outside a terminal's bounds cannot be used.
const SIZE_OUT_OF_BOUNDS: &str = "the pane's size is out of bounds";
/// Why a blank pane id cannot be used.
const BLANK_PANE_ID: &str = "the pane id is blank";
/// Appended when the pane muster could not run in was closed.
const PANE_CLOSED: &str = "; the pane was closed";
/// Appended, before the close's failure, when it was not.
const PANE_LEFT_OPEN: &str = "; the pane is left open, as closing it failed too:";

/// The herdr the caller's pane runs in, driven through its CLI.
#[derive(Debug)]
pub struct HerdrWorkspace {
    /// The herdr binary.
    program: PathBuf,
    /// The pane muster runs in, beside which panes open.
    caller: WorkspacePaneId,
    /// How long one call may take.
    timeout: Duration,
}

impl HerdrWorkspace {
    /// The herdr workspace of the caller's pane, when `env` names one
    /// (`HERDR_PANE_ID`, not blank). The binary is `HERDR_BIN_PATH` when it
    /// is absolute, else `herdr` on `PATH`.
    pub fn from_env(env: impl Fn(&str) -> Option<OsString>) -> Option<Self> {
        let caller = WorkspacePaneId::try_new(env(HERDR_PANE_ENV)?.into_string().ok()?).ok()?;
        let program = env(HERDR_BIN_ENV)
            .map(PathBuf::from)
            .filter(|program| program.is_absolute())
            .unwrap_or_else(|| PathBuf::from(HERDR_PROGRAM));
        Some(Self {
            program,
            caller,
            timeout: HERDR_CALL_TIMEOUT,
        })
    }

    /// A workspace on `program` for the pane `caller`, bounded by `timeout`.
    #[cfg(test)]
    pub fn with(program: PathBuf, caller: WorkspacePaneId, timeout: Duration) -> Self {
        Self {
            program,
            caller,
            timeout,
        }
    }

    /// Runs herdr with `args` and reads its answer as `T`. `action` names
    /// what was asked in an error; `unanswered` is appended to a timeout.
    ///
    /// # Errors
    /// Returns the call's failure, or [`WorkspaceError::Unreadable`] for an
    /// answer that is not a `T`.
    fn ask<T: DeserializeOwned>(
        &self,
        args: &[OsString],
        action: &str,
        unanswered: Option<&str>,
    ) -> Result<T, WorkspaceError> {
        let answer = self.run(args, action, unanswered)?;
        serde_json::from_str(&answer).map_err(|error| unreadable(action, &error.to_string()))
    }

    /// Runs herdr with `args`, returning what it printed.
    ///
    /// # Errors
    /// Returns the call's failure as the port reports it.
    fn run(
        &self,
        args: &[OsString],
        action: &str,
        unanswered: Option<&str>,
    ) -> Result<String, WorkspaceError> {
        call(&self.program, args, self.timeout)
            .map_err(|failure: HerdrFailure| failure.into_workspace_error(action, unanswered))
    }

    /// Which way to split, from the caller's size in its tab's layout.
    ///
    /// # Errors
    /// Returns the layout call's failure, or [`WorkspaceError::Unreadable`]
    /// when the layout does not list the caller at a usable size.
    fn direction_from_layout(&self) -> Result<SplitDirection, WorkspaceError> {
        let action = format!("{LAYOUT_ACTION} {}", self.caller);
        let layout: LayoutReply = self.ask(
            &words(&[PANE, LAYOUT, PANE_FLAG, self.caller.as_ref()]),
            &action,
            None,
        )?;
        let (width, height) = layout
            .size_of(self.caller.as_ref())
            .ok_or_else(|| unreadable(&action, CALLER_NOT_IN_LAYOUT))?;
        match (Cols::try_new(width), Rows::try_new(height)) {
            (Ok(cols), Ok(rows)) => Ok(SplitDirection::for_area(cols, rows)),
            _ => Err(unreadable(&action, SIZE_OUT_OF_BOUNDS)),
        }
    }

    /// Splits the caller's pane `direction`-wise into a shell in `request`'s
    /// folder holding its command's words, returning the new pane.
    ///
    /// # Errors
    /// Returns the split call's failure, or [`WorkspaceError::Unreadable`]
    /// for an answer naming no pane; a timeout or an unreadable answer notes
    /// that a pane may have opened.
    fn split(
        &self,
        request: &PaneRequest,
        direction: SplitDirection,
        env: Vec<(String, OsString)>,
    ) -> Result<WorkspacePaneId, WorkspaceError> {
        let action = format!("{SPLIT_ACTION} {}", self.caller);
        let mut args = words(&[
            PANE,
            SPLIT,
            self.caller.as_ref(),
            DIRECTION_FLAG,
            direction.into(),
            CWD_FLAG,
        ]);
        args.push(request.cwd().as_os_str().to_os_string());
        for (name, value) in env {
            let mut pair = OsString::from(name);
            pair.push(ENV_JOIN);
            pair.push(value);
            args.push(OsString::from(ENV_FLAG));
            args.push(pair);
        }
        args.push(OsString::from(match request.focus() {
            PaneFocus::Take => FOCUS_FLAG,
            PaneFocus::Keep => NO_FOCUS_FLAG,
        }));
        self.ask::<PaneReply>(&args, &action, Some(SPLIT_MAY_HAVE_OPENED))
            .and_then(|reply| pane_id(&action, &reply.into_pane_id()))
            .map_err(|error| match error {
                WorkspaceError::Unreadable {
                    workspace,
                    action,
                    reason,
                } => WorkspaceError::Unreadable {
                    workspace,
                    action,
                    reason: format!("{reason}{SPLIT_MAY_HAVE_OPENED}"),
                },
                other => other,
            })
    }

    /// Types `line` into `pane`; when that fails, closes `pane`.
    ///
    /// # Errors
    /// Returns [`WorkspaceError::RunFailed`], saying whether the pane was
    /// closed.
    fn type_line(&self, pane: &WorkspacePaneId, line: String) -> Result<(), WorkspaceError> {
        let action = format!("{RUN_ACTION} {pane}");
        let mut args = words(&[PANE, RUN, pane.as_ref()]);
        args.push(OsString::from(line));
        let Err(cause) = self.run(&args, &action, None) else {
            return Ok(());
        };
        let close_action = format!("{CLOSE_ACTION} {pane}");
        let closed = match self.run(&words(&[PANE, CLOSE, pane.as_ref()]), &close_action, None) {
            Ok(_) => PANE_CLOSED.to_string(),
            Err(error) => format!("{PANE_LEFT_OPEN} {error}"),
        };
        Err(WorkspaceError::RunFailed {
            workspace: WORKSPACE_NAME.to_string(),
            pane: pane.clone(),
            cause: Box::new(cause),
            closed,
        })
    }
}

impl TerminalWorkspace for HerdrWorkspace {
    fn open_beside(&self, request: &PaneRequest) -> Result<OpenedPane, WorkspaceError> {
        let direction = match request.direction() {
            Some(direction) => direction,
            None => self.direction_from_layout()?,
        };
        let (env, line) = typed_line(request.command());
        let pane = self.split(request, direction, env)?;
        self.type_line(&pane, line)?;
        Ok(OpenedPane::builder()
            .pane(pane)
            .direction(direction)
            .build())
    }

    fn check(&self) -> Result<WorkspaceProbe, WorkspaceError> {
        let action = format!("{GET_ACTION} {}", self.caller);
        let reply: PaneReply =
            self.ask(&words(&[PANE, GET, self.caller.as_ref()]), &action, None)?;
        Ok(WorkspaceProbe::builder()
            .workspace(WORKSPACE_NAME.to_string())
            .pane(pane_id(&action, &reply.into_pane_id())?)
            .build())
    }
}

/// `args` as arguments to a command.
fn words(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

/// The pane `id` herdr answered `action` with, made safe to print.
///
/// # Errors
/// Returns [`WorkspaceError::Unreadable`] for a blank id.
fn pane_id(action: &str, id: &str) -> Result<WorkspacePaneId, WorkspaceError> {
    WorkspacePaneId::try_new(herdr_text(id)).map_err(|_| unreadable(action, BLANK_PANE_ID))
}

/// herdr's answer to `action` could not be used, for `reason`.
fn unreadable(action: &str, reason: &str) -> WorkspaceError {
    WorkspaceError::Unreadable {
        workspace: WORKSPACE_NAME.to_string(),
        action: herdr_text(action),
        reason: herdr_text(reason),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Instant};

    use super::*;
    use crate::{
        adapter::{
            herdr::fake_herdr::{CALLER, FakeHerdr},
            signal_target::SignalTarget,
            test_wait::wait_until,
        },
        domain::terminal_workspace::{PaneCommand, PaneFocus, SplitDirection},
    };

    /// A timeout no fake call comes near.
    const AMPLE: Duration = Duration::from_secs(20);
    /// A timeout a hung fake call runs past, long enough for the fake to
    /// start under load.
    const SHORT: Duration = Duration::from_millis(1500);
    /// The pane the fake's split opens.
    const NEW_PANE: &str = "w1:p2";
    /// A layout in which the caller is wide.
    const WIDE_LAYOUT: &str = r#"{"id":"1","result":{"type":"pane_layout","layout":{"panes":[
        {"pane_id":"w1:p9","focused":false,"rect":{"x":0,"y":0,"width":10,"height":10}},
        {"pane_id":"w1:p1","focused":true,"rect":{"x":0,"y":0,"width":200,"height":50}}]}}}"#;
    /// The split's answer.
    const SPLIT_REPLY: &str =
        r#"{"id":"1","result":{"type":"pane_info","pane":{"pane_id":"w1:p2"}}}"#;
    /// A refusal as herdr prints it.
    const REFUSAL: &str = r#"{"id":"1","error":{"code":"pane_not_found","message":"no pane"}}"#;

    /// An environment holding exactly `pairs`.
    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), OsString::from(value)))
            .collect();
        move |key| map.get(key).cloned()
    }

    /// A request for a pane in `cwd` running `program` with `args`.
    fn request(
        cwd: &str,
        program: &str,
        args: &[&str],
        focus: PaneFocus,
        direction: Option<SplitDirection>,
    ) -> PaneRequest {
        PaneRequest::builder()
            .cwd(PathBuf::from(cwd))
            .command(
                PaneCommand::builder()
                    .program(PathBuf::from(program))
                    .args(args.iter().map(OsString::from).collect())
                    .build(),
            )
            .focus(focus)
            .direction(direction)
            .build()
    }

    /// An attach request in `/srv/app`, taking the focus.
    fn attach_request(direction: Option<SplitDirection>) -> PaneRequest {
        request(
            "/srv/app",
            "/bin/muster",
            &["--project", "/srv/app", "attach", "--", "api"],
            PaneFocus::Take,
            direction,
        )
    }

    /// `args` as owned argv.
    fn argv(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    /// The typed line for a command of `words` words.
    fn line_of(words: usize) -> String {
        let refs: Vec<String> = (0..words)
            .map(|index| format!("\"$MUSTER_OPEN_ARG_{index}\""))
            .collect();
        format!(" {} && exit", refs.join(" "))
    }

    /// A fake that splits and runs successfully.
    fn splitting() -> FakeHerdr {
        let herdr = FakeHerdr::new();
        herdr.answer("layout", WIDE_LAYOUT);
        herdr.answer("split", SPLIT_REPLY);
        herdr
    }

    /// The pane is read only when the pane id is set and not blank.
    #[test]
    fn from_env_needs_a_non_empty_pane_id() {
        assert!(HerdrWorkspace::from_env(env_of(&[])).is_none());
        assert!(HerdrWorkspace::from_env(env_of(&[("HERDR_PANE_ID", "  ")])).is_none());
        let workspace = HerdrWorkspace::from_env(env_of(&[("HERDR_PANE_ID", "w1:p3")])).unwrap();
        assert_eq!(workspace.caller.as_ref(), "w1:p3");
        assert_eq!(workspace.timeout, HERDR_CALL_TIMEOUT);
    }

    /// An absolute `HERDR_BIN_PATH` is the binary; a relative one is
    /// ignored for `herdr` on `PATH`.
    #[test]
    fn from_env_prefers_an_absolute_herdr_bin_path_and_ignores_a_relative_one() {
        let pane = ("HERDR_PANE_ID", "w1:p3");
        let named = |bin: &str| {
            HerdrWorkspace::from_env(env_of(&[pane, ("HERDR_BIN_PATH", bin)]))
                .unwrap()
                .program
        };
        assert_eq!(
            named("/opt/herdr/bin/herdr"),
            PathBuf::from("/opt/herdr/bin/herdr")
        );
        assert_eq!(named("bin/herdr"), PathBuf::from("herdr"));
        assert_eq!(
            HerdrWorkspace::from_env(env_of(&[pane])).unwrap().program,
            PathBuf::from("herdr")
        );
    }

    /// Without a direction, the caller's size picks it, the split names the
    /// caller, the folder and each word, and the line is typed into the new
    /// pane.
    #[test]
    fn open_reads_the_layout_splits_beside_the_caller_and_types_the_line() {
        let herdr = splitting();
        let opened = herdr
            .workspace(AMPLE)
            .open_beside(&attach_request(None))
            .unwrap();
        assert_eq!(opened.pane().as_ref(), NEW_PANE);
        assert_eq!(opened.direction(), SplitDirection::Right);
        assert_eq!(herdr.calls(), [
            argv(&["pane", "layout", "--pane", CALLER]),
            argv(&[
                "pane",
                "split",
                CALLER,
                "--direction",
                "right",
                "--cwd",
                "/srv/app",
                "--env",
                "MUSTER_OPEN_ARG_0=/bin/muster",
                "--env",
                "MUSTER_OPEN_ARG_1=--project",
                "--env",
                "MUSTER_OPEN_ARG_2=/srv/app",
                "--env",
                "MUSTER_OPEN_ARG_3=attach",
                "--env",
                "MUSTER_OPEN_ARG_4=--",
                "--env",
                "MUSTER_OPEN_ARG_5=api",
                "--focus",
            ]),
            argv(&["pane", "run", NEW_PANE, &line_of(6)]),
        ]);
    }

    /// A pane the layout shows too narrow to halve splits down.
    #[test]
    fn a_narrow_caller_splits_down() {
        let herdr = splitting();
        herdr.answer(
            "layout",
            r#"{"result":{"layout":{"panes":[{"pane_id":"w1:p1","rect":{"width":79,"height":10}}]}}}"#,
        );
        let opened = herdr
            .workspace(AMPLE)
            .open_beside(&attach_request(None))
            .unwrap();
        assert_eq!(opened.direction(), SplitDirection::Down);
        assert_eq!(herdr.calls()[1][4], "down");
    }

    /// A layout that does not list the caller cannot place the pane.
    #[test]
    fn a_layout_without_the_caller_is_unreadable() {
        let herdr = splitting();
        herdr.answer(
            "layout",
            r#"{"result":{"layout":{"panes":[{"pane_id":"w1:p9","rect":{"width":79,"height":10}}]}}}"#,
        );
        let error = herdr
            .workspace(AMPLE)
            .open_beside(&attach_request(None))
            .unwrap_err();
        assert!(
            matches!(error, WorkspaceError::Unreadable { .. }),
            "{error}"
        );
        assert_eq!(herdr.calls().len(), 1);
    }

    /// A direction asked for is used as is, without reading the layout.
    #[test]
    fn a_given_direction_skips_the_layout_call() {
        let herdr = splitting();
        let opened = herdr
            .workspace(AMPLE)
            .open_beside(&attach_request(Some(SplitDirection::Down)))
            .unwrap();
        assert_eq!(opened.direction(), SplitDirection::Down);
        let calls = herdr.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0][..5],
            argv(&["pane", "split", CALLER, "--direction", "down"])
        );
    }

    /// Taking the focus passes `--focus`; keeping it passes `--no-focus`.
    #[test]
    fn take_focus_passes_focus_and_keep_passes_no_focus() {
        for (focus, flag) in [
            (PaneFocus::Take, "--focus"),
            (PaneFocus::Keep, "--no-focus"),
        ] {
            let herdr = splitting();
            let asked = request(
                "/srv",
                "/bin/muster",
                &[],
                focus,
                Some(SplitDirection::Right),
            );
            herdr.workspace(AMPLE).open_beside(&asked).unwrap();
            let split = &herdr.calls()[0];
            assert_eq!(split.last().unwrap(), flag);
            let flags = ["--focus", "--no-focus"];
            assert_eq!(
                split
                    .iter()
                    .filter(|arg| flags.contains(&arg.to_str().unwrap()))
                    .count(),
                1
            );
        }
    }

    /// Names and paths that would each break a typed line reach herdr only
    /// as `--env` values and `--cwd`; the typed line is the fixed one.
    #[test]
    fn hostile_names_and_paths_travel_only_as_env_values() {
        let hostile = ["'", "\"", "$(id)", "`id`", ";", "\n", "-x"].concat();
        let herdr = splitting();
        let asked = request(
            "/tmp/a b",
            "/opt/my muster/muster",
            &["--project", "/tmp/a b", "attach", "--", &hostile],
            PaneFocus::Keep,
            Some(SplitDirection::Right),
        );
        herdr.workspace(AMPLE).open_beside(&asked).unwrap();
        let calls = herdr.calls();
        let split = &calls[0];
        assert_eq!(split[5..7], argv(&["--cwd", "/tmp/a b"]));
        assert!(split.contains(&OsString::from("MUSTER_OPEN_ARG_0=/opt/my muster/muster")));
        assert!(split.contains(&OsString::from(format!("MUSTER_OPEN_ARG_5={hostile}"))));
        assert_eq!(calls[1], argv(&["pane", "run", NEW_PANE, &line_of(6)]));
        for call in &calls[1..] {
            for arg in call {
                let arg = arg.to_str().unwrap();
                assert!(!arg.contains("$(id)") && !arg.contains("a b"), "{arg}");
            }
        }
    }

    /// herdr's JSON refusal is reported with its code and message, naming
    /// what was asked.
    #[test]
    fn a_herdr_error_reply_is_refused_with_its_code_and_message() {
        let herdr = FakeHerdr::new();
        herdr.refuse("get", REFUSAL, 1);
        let error = herdr.workspace(AMPLE).check().unwrap_err();
        let WorkspaceError::Refused {
            workspace,
            action,
            code,
            message,
        } = &error
        else {
            panic!("{error}");
        };
        assert_eq!(workspace, "herdr");
        assert!(action.contains(CALLER), "{action}");
        assert_eq!(code, "pane_not_found");
        assert_eq!(message, "no pane");
    }

    /// A code and message holding control characters reach the caller as
    /// printable text, each cut to a bound.
    #[test]
    fn herdr_text_with_control_characters_is_sanitized_and_bounded() {
        let herdr = FakeHerdr::new();
        let flood = "y".repeat(5000);
        herdr.refuse(
            "get",
            &format!(
                r#"{{"error":{{"code":"x\u001b]0;pwned\u0007","message":"a\u001b[2J{flood}"}}}}"#
            ),
            1,
        );
        let error = herdr.workspace(AMPLE).check().unwrap_err();
        let WorkspaceError::Refused { code, message, .. } = &error else {
            panic!("{error}");
        };
        assert_eq!(code, "x\\u{1b}]0;pwned\\u{7}");
        assert!(message.starts_with("a\\u{1b}[2Jyyy"), "{message}");
        assert!(message.chars().count() < 600, "{}", message.len());
        assert!(!error.to_string().chars().any(char::is_control));
    }

    /// A refusal printed as plain text, as herdr's argument parser does,
    /// is reported with the exit status and the text.
    #[test]
    fn a_syntax_error_exit_without_json_is_refused_with_its_text() {
        let herdr = FakeHerdr::new();
        herdr.refuse("split", "error: unexpected argument '--env' found\n", 2);
        let error = herdr
            .workspace(AMPLE)
            .open_beside(&attach_request(Some(SplitDirection::Right)))
            .unwrap_err();
        let WorkspaceError::Refused { code, message, .. } = &error else {
            panic!("{error}");
        };
        assert!(code.contains('2'), "{code}");
        assert_eq!(message, "error: unexpected argument '--env' found");
        assert_eq!(herdr.calls().len(), 1);
    }

    /// When typing into the new pane fails, the pane is closed and the run's
    /// failure reported.
    #[test]
    fn a_failed_run_closes_the_pane_it_split() {
        let herdr = splitting();
        herdr.refuse("run", REFUSAL, 1);
        let error = herdr
            .workspace(AMPLE)
            .open_beside(&attach_request(Some(SplitDirection::Right)))
            .unwrap_err();
        let WorkspaceError::RunFailed {
            pane,
            cause,
            closed,
            ..
        } = &error
        else {
            panic!("{error}");
        };
        assert_eq!(pane.as_ref(), NEW_PANE);
        assert!(matches!(**cause, WorkspaceError::Refused { .. }), "{cause}");
        assert!(closed.contains("closed"), "{closed}");
        assert_eq!(
            herdr.calls().last().unwrap(),
            &argv(&["pane", "close", NEW_PANE])
        );
    }

    /// When closing the pane fails too, the error says it is left open.
    #[test]
    fn a_failed_close_after_a_failed_run_says_the_pane_is_left() {
        let herdr = splitting();
        herdr.refuse("run", REFUSAL, 1);
        herdr.refuse("close", REFUSAL, 1);
        let error = herdr
            .workspace(AMPLE)
            .open_beside(&attach_request(Some(SplitDirection::Right)))
            .unwrap_err();
        let WorkspaceError::RunFailed { closed, .. } = &error else {
            panic!("{error}");
        };
        assert!(closed.contains("left open"), "{closed}");
        assert!(closed.contains("pane_not_found"), "{closed}");
    }

    /// A herdr that never answers is killed with what it started and reaped
    /// at the timeout, and reported unreachable.
    #[test]
    fn a_hung_herdr_is_killed_and_reaped_at_the_timeout() {
        let herdr = FakeHerdr::new();
        herdr.hang("get");
        let started = Instant::now();
        let error = herdr.workspace(SHORT).check().unwrap_err();
        assert!(started.elapsed() < AMPLE, "{:?}", started.elapsed());
        assert!(
            matches!(error, WorkspaceError::Unreachable { .. }),
            "{error}"
        );
        let (pid, sleeper) = herdr.pids("get");
        assert!(
            !SignalTarget::Process(pid).is_alive(),
            "herdr was left a zombie or running"
        );
        let sleeper = sleeper.expect("the fake herdr never started its sleep");
        wait_until("the sleep herdr started to die", || {
            !SignalTarget::Process(sleeper).is_alive()
        });
    }

    /// A split that timed out, or succeeded with an answer naming no pane,
    /// says a pane may have opened, and nothing is typed.
    #[test]
    fn a_hung_or_unreadable_split_says_a_pane_may_be_left() {
        let herdr = FakeHerdr::new();
        herdr.hang("split");
        let error = herdr
            .workspace(SHORT)
            .open_beside(&attach_request(Some(SplitDirection::Right)))
            .unwrap_err();
        assert!(error.to_string().contains("may have opened"), "{error}");

        for reply in ["not json", r#"{"result":{"pane":{"pane_id":""}}}"#] {
            let herdr = FakeHerdr::new();
            herdr.answer("split", reply);
            let error = herdr
                .workspace(AMPLE)
                .open_beside(&attach_request(Some(SplitDirection::Right)))
                .unwrap_err();
            assert!(
                matches!(error, WorkspaceError::Unreadable { .. }),
                "{error}"
            );
            assert!(error.to_string().contains("may have opened"), "{error}");
            assert_eq!(herdr.calls().len(), 1);
        }
    }

    /// A binary that does not exist is unreachable.
    #[test]
    fn a_missing_herdr_binary_is_unreachable() {
        let herdr = FakeHerdr::new();
        let missing = HerdrWorkspace::with(
            herdr.program().with_file_name("no-such-herdr"),
            WorkspacePaneId::try_new(CALLER).unwrap(),
            AMPLE,
        );
        let error = missing.check().unwrap_err();
        assert!(
            matches!(error, WorkspaceError::Unreachable { .. }),
            "{error}"
        );
    }

    /// An answer that is not the expected JSON, or that is too large, cannot
    /// be read.
    #[test]
    fn an_unparseable_or_oversized_reply_is_unreadable() {
        let herdr = FakeHerdr::new();
        herdr.answer("get", "not json");
        let error = herdr.workspace(AMPLE).check().unwrap_err();
        assert!(
            matches!(error, WorkspaceError::Unreadable { .. }),
            "{error}"
        );

        herdr.answer("get", r#"{"result":{"pane":{}}}"#);
        let error = herdr.workspace(AMPLE).check().unwrap_err();
        assert!(
            matches!(error, WorkspaceError::Unreadable { .. }),
            "{error}"
        );

        herdr.answer("get", r#"{"result":{"pane":{"pane_id":"  "}}}"#);
        let error = herdr.workspace(AMPLE).check().unwrap_err();
        assert!(
            matches!(error, WorkspaceError::Unreadable { .. }),
            "{error}"
        );

        let oversized = format!("{SPLIT_REPLY}{}", " ".repeat(1024 * 1024));
        herdr.answer("get", &oversized);
        let error = herdr.workspace(AMPLE).check().unwrap_err();
        assert!(
            matches!(error, WorkspaceError::Unreadable { .. }),
            "{error}"
        );
    }

    /// Fields muster does not read are ignored.
    #[test]
    fn a_reply_with_extra_fields_still_reads() {
        let herdr = FakeHerdr::new();
        herdr.answer(
            "get",
            r#"{"id":"7","future":[1,2],"result":{"type":"pane_info","pane":
                {"pane_id":"w1:p1","focused":true,"tab_id":"w1:t1","new":{}}}}"#,
        );
        assert_eq!(
            herdr.workspace(AMPLE).check().unwrap().pane().as_ref(),
            CALLER
        );
    }

    /// The check asks herdr about the caller's pane and names herdr and the
    /// pane, sanitized.
    #[test]
    fn check_reads_the_caller_pane() {
        let herdr = FakeHerdr::new();
        herdr.answer(
            "get",
            r#"{"result":{"type":"pane_info","pane":{"pane_id":"w1:p1\u001b[2J"}}}"#,
        );
        let probe = herdr.workspace(AMPLE).check().unwrap();
        assert_eq!(probe.workspace(), "herdr");
        assert_eq!(probe.pane().as_ref(), "w1:p1\\u{1b}[2J");
        assert_eq!(herdr.calls(), [argv(&["pane", "get", CALLER])]);
    }
}

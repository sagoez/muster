//! A stand-in `herdr` for tests: a shell script that logs each call's
//! arguments and answers from canned files, so no test reaches a real herdr.

use std::{
    ffi::OsString,
    fs,
    io::ErrorKind,
    os::unix::{ffi::OsStringExt, fs::PermissionsExt},
    path::PathBuf,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use tempfile::TempDir;

use super::herdr_workspace::HerdrWorkspace;
use crate::{
    adapter::test_wait::{POLL, WAIT},
    domain::value::{Pid, WorkspacePaneId},
};

/// The fake's file name.
const SCRIPT: &str = "herdr";
/// The log of every call's arguments.
const ARGV_LOG: &str = "argv.log";
/// Ends each call's record in the log; each argument ends in a NUL.
const RECORD_END: u8 = 0x1E;
/// The argument that makes the fake exit at once without logging, to check
/// it can be run.
const SETTLE_ARG: &str = "settle";
/// How long a hung call sleeps, far past any test's timeout.
const HANG_SECONDS: &str = "30";
/// The pane the fake's caller runs in.
pub const CALLER: &str = "w1:p1";
/// Mode of the fake script: owner may run it.
const SCRIPT_MODE: u32 = 0o700;
/// The fake itself. Each call is answered from `<pane>-<verb>.out`, `.err`,
/// `.status` and `.sleep`, and records its pid in `.pid`.
const SCRIPT_BODY: &str = r#"#!/bin/sh
dir=$(dirname "$0")
[ "$1" = settle ] && exit 0
printf '%s\0' "$@" >> "$dir/argv.log"
printf '\036' >> "$dir/argv.log"
key="$1-$2"
echo $$ > "$dir/$key.pid"
if [ -f "$dir/$key.sleep" ]; then
  sleep "$(cat "$dir/$key.sleep")" &
  echo $! > "$dir/$key.sleeper"
  wait
fi
[ -f "$dir/$key.out" ] && cat "$dir/$key.out"
[ -f "$dir/$key.err" ] && cat "$dir/$key.err" >&2
[ -f "$dir/$key.status" ] && exit "$(cat "$dir/$key.status")"
exit 0
"#;

/// A fake `herdr` in a scratch folder of its own.
pub struct FakeHerdr {
    /// The folder holding the script, its answers and its log.
    dir: TempDir,
}

impl FakeHerdr {
    /// Writes the fake and waits until it can be run: a script just written
    /// cannot be run while another test's fork still holds it open.
    pub fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let script = dir.path().join(SCRIPT);
        fs::write(&script, SCRIPT_BODY).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(SCRIPT_MODE)).unwrap();
        let started = Instant::now();
        loop {
            match Command::new(&script).arg(SETTLE_ARG).status() {
                Ok(status) => {
                    assert!(status.success());
                    break;
                },
                Err(error) if error.kind() == ErrorKind::ExecutableFileBusy => {
                    assert!(started.elapsed() < WAIT, "the fake herdr stays busy");
                    thread::sleep(POLL);
                },
                Err(error) => panic!("the fake herdr does not run: {error}"),
            }
        }
        Self { dir }
    }

    /// The fake's path.
    pub fn program(&self) -> PathBuf {
        self.dir.path().join(SCRIPT)
    }

    /// A workspace on this fake for the pane [`CALLER`], bounded by
    /// `timeout`.
    pub fn workspace(&self, timeout: Duration) -> HerdrWorkspace {
        HerdrWorkspace::with(
            self.program(),
            WorkspacePaneId::try_new(CALLER).unwrap(),
            timeout,
        )
    }

    /// Makes `herdr pane <verb>` print `stdout` and exit 0.
    pub fn answer(&self, verb: &str, stdout: &str) {
        fs::write(self.file(verb, "out"), stdout).unwrap();
    }

    /// Makes `herdr pane <verb>` print `stderr` and exit with `status`.
    pub fn refuse(&self, verb: &str, stderr: &str, status: u8) {
        fs::write(self.file(verb, "err"), stderr).unwrap();
        fs::write(self.file(verb, "status"), status.to_string()).unwrap();
    }

    /// Makes `herdr pane <verb>` sleep far past any test's timeout, in a
    /// child of its own.
    pub fn hang(&self, verb: &str) {
        fs::write(self.file(verb, "sleep"), HANG_SECONDS).unwrap();
    }

    /// The pid `herdr pane <verb>` ran as, and that of the sleep it
    /// started, if it hung.
    pub fn pids(&self, verb: &str) -> (Pid, Option<Pid>) {
        let read = |kind: &str| {
            fs::read_to_string(self.file(verb, kind))
                .ok()
                .map(|pid| Pid::try_new(pid.trim().parse().unwrap()).unwrap())
        };
        (
            read("pid").expect("the fake herdr never ran"),
            read("sleeper"),
        )
    }

    /// Every call's arguments, in order.
    pub fn calls(&self) -> Vec<Vec<OsString>> {
        let Ok(log) = fs::read(self.dir.path().join(ARGV_LOG)) else {
            return Vec::new();
        };
        log.split(|byte| *byte == RECORD_END)
            .filter(|record| !record.is_empty())
            .map(|record| {
                record
                    .strip_suffix(&[0])
                    .unwrap()
                    .split(|byte| *byte == 0)
                    .map(|arg| OsString::from_vec(arg.to_vec()))
                    .collect()
            })
            .collect()
    }

    /// The canned file of `kind` for `herdr pane <verb>`.
    fn file(&self, verb: &str, kind: &str) -> PathBuf {
        self.dir.path().join(format!("pane-{verb}.{kind}"))
    }
}

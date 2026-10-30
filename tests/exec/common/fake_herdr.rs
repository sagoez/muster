use std::{
    ffi::OsString,
    fs,
    io::ErrorKind,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::PermissionsExt,
    },
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

use super::{
    herdr_env::{HERDR_BIN_ENV, HERDR_PANE_ENV},
    scratch::Scratch,
    wait::{POLL, WAIT},
};

/// The folder inside the scratch the fake lives in.
const FAKE_DIR: &str = "fake-herdr";
/// The fake's file name.
const SCRIPT: &str = "herdr";
/// The log of every call's arguments.
const ARGV_LOG: &str = "argv.log";
/// Ends each call's record in the log; each argument ends in a NUL.
const RECORD_END: u8 = 0x1E;
/// The argument that makes the fake exit at once without logging, to check
/// it can be run.
const SETTLE_ARG: &str = "settle";
/// Mode of the fake script: owner may run it.
const SCRIPT_MODE: u32 = 0o700;
/// The pane the fake's caller runs in.
pub const CALLER: &str = "w1:p1";
/// The pane the fake's split opens.
pub const OPENED: &str = "w1:p2";
/// The fake itself. Each `<group> <verb>` call is answered from
/// `<group>-<verb>.out`, `.err` and `.status`, after sleeping as `.delay`
/// says, and records its pid in `.pid`. With `.fork` it instead starts a
/// child sleeping that long, records the child's pid in `.grandchild` and
/// waits on it, as a wrapper around herdr might.
const SCRIPT_BODY: &str = r#"#!/bin/sh
dir=$(dirname "$0")
[ "$1" = settle ] && exit 0
printf '%s\0' "$@" >> "$dir/argv.log"
printf '\036' >> "$dir/argv.log"
key="$1-$2"
echo $$ > "$dir/$key.pid"
[ -f "$dir/$key.delay" ] && sleep "$(cat "$dir/$key.delay")"
if [ -f "$dir/$key.fork" ]; then
  sleep "$(cat "$dir/$key.fork")" &
  echo $! > "$dir/$key.grandchild"
  wait
  exit 0
fi
[ -f "$dir/$key.out" ] && cat "$dir/$key.out"
[ -f "$dir/$key.err" ] && cat "$dir/$key.err" >&2
[ -f "$dir/$key.status" ] && exit "$(cat "$dir/$key.status")"
exit 0
"#;
/// The answers every fake starts with: the caller wide in its layout, a
/// split that opens [`OPENED`], and a run, close and get that succeed.
const ANSWERS: [(&str, &str); 5] = [
    (
        "pane-layout",
        r#"{"id":"1","result":{"type":"pane_layout","layout":{"panes":[{"pane_id":"w1:p1","focused":true,"rect":{"x":0,"y":0,"width":200,"height":50}}]}}}"#,
    ),
    (
        "pane-split",
        r#"{"id":"1","result":{"type":"pane_info","pane":{"pane_id":"w1:p2"}}}"#,
    ),
    ("pane-run", r#"{"id":"1","result":{"type":"ok"}}"#),
    ("pane-close", r#"{"id":"1","result":{"type":"ok"}}"#),
    (
        "pane-get",
        r#"{"id":"1","result":{"type":"pane_info","pane":{"pane_id":"w1:p1"}}}"#,
    ),
];
/// The answer file's extension for what a call prints.
const OUT: &str = "out";
/// The answer file's extension for what a call prints on stderr.
const ERR: &str = "err";
/// The answer file's extension for a call's exit status.
const STATUS: &str = "status";
/// The file extension a call records its pid in.
const PID: &str = "pid";
/// The answer file's extension for how long a call's child sleeps while
/// the call waits on it.
const FORK: &str = "fork";
/// The file extension a call records its child's pid in.
const GRANDCHILD: &str = "grandchild";
/// The answer file's extension for how long a call sleeps before it
/// answers.
const DELAY: &str = "delay";
/// How long a hung call sleeps, far past any test.
const HANG_SECONDS: &str = "300";
/// Joins a variable's name to its value in a split's `--env`.
const ENV_JOIN: u8 = b'=';
/// The split's flag naming a variable of the new pane's shell.
const ENV_FLAG: &str = "--env";
/// The split's flag naming the folder the new pane's shell starts in.
const CWD_FLAG: &str = "--cwd";
/// The verb that opens a pane.
const SPLIT_VERB: &str = "split";
/// The verb that types a line into a pane.
const RUN_VERB: &str = "run";

/// A stand-in `herdr` in a scratch folder: a shell script that logs each
/// call's arguments and answers from canned files, so no test reaches a
/// real herdr.
pub struct FakeHerdr {
    /// The folder holding the script, its answers and its log.
    dir: PathBuf,
}

impl FakeHerdr {
    /// Writes the fake inside `scratch` with its default answers, and waits
    /// until it can be run: a script just written cannot be run while
    /// another test's fork still holds it open.
    pub fn new(scratch: &Scratch) -> Self {
        let fake = Self {
            dir: scratch.path().join(FAKE_DIR),
        };
        fs::create_dir_all(&fake.dir).unwrap();
        for (call, answer) in ANSWERS {
            fs::write(fake.file(call, OUT), answer).unwrap();
        }
        let script = fake.program();
        fs::write(&script, SCRIPT_BODY).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(SCRIPT_MODE)).unwrap();
        let started = Instant::now();
        loop {
            match Command::new(&script).arg(SETTLE_ARG).status() {
                Ok(status) => {
                    assert!(status.success());
                    return fake;
                },
                Err(error) if error.kind() == ErrorKind::ExecutableFileBusy => {
                    assert!(started.elapsed() < WAIT, "the fake herdr stays busy");
                    thread::sleep(POLL);
                },
                Err(error) => panic!("the fake herdr does not run: {error}"),
            }
        }
    }

    /// The fake's script.
    pub fn program(&self) -> PathBuf {
        self.dir.join(SCRIPT)
    }

    /// Runs `command` as if in the pane [`CALLER`] of a herdr whose binary
    /// is `program`.
    pub fn place<'a>(command: &'a mut Command, program: &Path) -> &'a mut Command {
        command
            .env(HERDR_PANE_ENV, CALLER)
            .env(HERDR_BIN_ENV, program)
    }

    /// Makes the `call` (`pane-run`, ...) fail with `stderr` and `status`.
    pub fn refuse(&self, call: &str, stderr: &str, status: u8) {
        fs::remove_file(self.file(call, OUT)).unwrap();
        fs::write(self.file(call, ERR), stderr).unwrap();
        fs::write(self.file(call, STATUS), status.to_string()).unwrap();
    }

    /// Makes the `call` answer only after `delay`, in whole seconds.
    pub fn delay(&self, call: &str, delay: Duration) {
        fs::write(self.file(call, DELAY), delay.as_secs().to_string()).unwrap();
    }

    /// Makes the `call` start a child that sleeps far past any test and
    /// wait on it instead of answering.
    pub fn fork_and_hang(&self, call: &str) {
        fs::write(self.file(call, FORK), HANG_SECONDS).unwrap();
    }

    /// The pid the `call` last ran as, once it has run.
    pub fn pid(&self, call: &str) -> Option<u32> {
        self.recorded(call, PID)
    }

    /// The pid of the child the `call` last started, once it has.
    pub fn grandchild(&self, call: &str) -> Option<u32> {
        self.recorded(call, GRANDCHILD)
    }

    /// The pid the `call` recorded in its file with `extension`, once it
    /// has.
    fn recorded(&self, call: &str, extension: &str) -> Option<u32> {
        fs::read_to_string(self.file(call, extension))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    /// Every call's arguments, in order.
    pub fn calls(&self) -> Vec<Vec<OsString>> {
        let log = fs::read(self.dir.join(ARGV_LOG)).unwrap_or_default();
        log.split(|byte| *byte == RECORD_END)
            .filter(|record| !record.is_empty())
            .map(|record| {
                let mut args: Vec<OsString> = record
                    .split(|byte| *byte == 0)
                    .map(|arg| OsString::from_vec(arg.to_vec()))
                    .collect();
                args.pop();
                args
            })
            .collect()
    }

    /// The folder and the variables the split asked the new pane's shell
    /// to start with, and the line typed into it, as a pane's shell gets
    /// them.
    pub fn pane_shell(&self) -> (PathBuf, Vec<(OsString, OsString)>, OsString) {
        let calls = self.calls();
        let split = calls
            .iter()
            .find(|call| call.get(1).is_some_and(|verb| verb == SPLIT_VERB))
            .expect("a pane was split");
        let typed = calls
            .iter()
            .find(|call| call.get(1).is_some_and(|verb| verb == RUN_VERB))
            .and_then(|call| call.last().cloned())
            .expect("a line was typed");
        let mut cwd = None;
        let mut env = Vec::new();
        for pair in split.windows(2) {
            if pair[0] == CWD_FLAG {
                cwd = Some(PathBuf::from(&pair[1]));
            }
            if pair[0] == ENV_FLAG {
                let bytes = pair[1].as_bytes();
                let join = bytes.iter().position(|byte| *byte == ENV_JOIN).unwrap();
                env.push((
                    OsString::from_vec(bytes[..join].to_vec()),
                    OsString::from_vec(bytes[join + 1..].to_vec()),
                ));
            }
        }
        (cwd.expect("the split named a folder"), env, typed)
    }

    /// The answer file of `call` with `extension`.
    fn file(&self, call: &str, extension: &str) -> PathBuf {
        self.dir.join(format!("{call}.{extension}"))
    }
}

use std::{
    ffi::OsString,
    io::{self, Read},
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, bounded};

use super::herdr_failure::HerdrFailure;
use crate::adapter::bounded_read::read_capped;

/// The most bytes muster reads of what herdr prints on either stream.
const MAX_REPLY_BYTES: u64 = 1024 * 1024;
/// How often a herdr that closed its output is checked for having exited.
const EXIT_POLL: Duration = Duration::from_millis(10);
/// Why a stream's reader ended without an answer.
const READER_LOST: &str = "the reader of herdr's output stopped";

/// Runs `program` with `args` and returns what it printed on stdout once it
/// exits 0, all within `timeout`. Past it the child is killed and reaped
/// before this returns, so a hung herdr costs at most `timeout` and leaves
/// no zombie. The child stays in muster's process group, so a Ctrl-C or a
/// hangup that ends muster ends it too.
///
/// # Errors
/// Returns a [`HerdrFailure`] for a binary that would not start, a run past
/// `timeout`, output past [`MAX_REPLY_BYTES`] or not UTF-8, or an
/// unsuccessful exit.
pub(super) fn call(
    program: &Path,
    args: &[OsString],
    timeout: Duration,
) -> Result<String, HerdrFailure> {
    let deadline = Instant::now() + timeout;
    let spawn_failed = |source| HerdrFailure::Spawn {
        program: program.to_path_buf(),
        source,
    };
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(spawn_failed)?;
    let stdout = child.stdout.take().map(drained);
    let stderr = child.stderr.take().map(drained);
    let (Some(stdout), Some(stderr)) = (stdout, stderr) else {
        stop(&mut child);
        return Err(spawn_failed(io::Error::other(READER_LOST)));
    };
    let Some(stdout) = answer_by(&stdout, deadline) else {
        stop(&mut child);
        return Err(HerdrFailure::TimedOut(timeout));
    };
    let Some(stderr) = answer_by(&stderr, deadline) else {
        stop(&mut child);
        return Err(HerdrFailure::TimedOut(timeout));
    };
    let status = exit_by(&mut child, deadline, timeout)?;
    if status.success() {
        stdout.map_err(HerdrFailure::Unreadable)
    } else {
        Err(HerdrFailure::Exited {
            status,
            stderr: stderr.unwrap_or_default(),
        })
    }
}

/// Reads `pipe` on a thread of its own, up to [`MAX_REPLY_BYTES`] and then
/// to its end, so herdr never blocks on a full pipe; the text comes back on
/// the returned channel.
fn drained(mut pipe: impl Read + Send + 'static) -> Receiver<io::Result<String>> {
    let (sent, read) = bounded(1);
    thread::spawn(move || {
        let text = read_capped(&mut pipe, MAX_REPLY_BYTES);
        let drained = io::copy(&mut pipe, &mut io::sink());
        let _ = sent.send(drained.and(text));
    });
    read
}

/// What `reader` read, once it reaches its pipe's end before `deadline`;
/// none past it.
fn answer_by(
    reader: &Receiver<io::Result<String>>,
    deadline: Instant,
) -> Option<io::Result<String>> {
    match reader.recv_deadline(deadline) {
        Ok(text) => Some(text),
        Err(error) if error.is_disconnected() => Some(Err(io::Error::other(READER_LOST))),
        Err(_) => None,
    }
}

/// How `child` exited, once it does before `deadline`; past it, the child is
/// stopped and reaped.
///
/// # Errors
/// Returns [`HerdrFailure::TimedOut`] past `deadline`, or
/// [`HerdrFailure::Wait`] when the child cannot be waited on.
fn exit_by(
    child: &mut Child,
    deadline: Instant,
    timeout: Duration,
) -> Result<ExitStatus, HerdrFailure> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {},
            Err(error) => {
                stop(child);
                return Err(HerdrFailure::Wait(error));
            },
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            stop(child);
            return Err(HerdrFailure::TimedOut(timeout));
        }
        thread::sleep(EXIT_POLL.min(left));
    }
}

/// Kills `child`, then reaps it. Only the child: it shares muster's process
/// group, and the herdr CLI starts nothing that outlives it.
fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

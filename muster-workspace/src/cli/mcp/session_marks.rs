//! The marks that make `muster mcp context` print once per session start,
//! when more than one of the agent's settings files runs muster's hook: the
//! first run for a session and source leaves a mark in muster's private
//! runtime folder, and a run that finds a fresh one prints nothing.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, ErrorKind},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use getset::Getters;
use muster_os::private_dir::{
    create_private_dir_all, is_owned_private_dir, prepare_socket_dir, socket_dir,
};
use muster_protocol::hash_key;
use typed_builder::TypedBuilder;

use crate::cli::mcp::hook_input::HookInput;

/// How long a mark holds: the hooks of one session start run together,
/// while a later start of the same kind, such as a second compaction, comes
/// well after.
pub const MARK_TTL: Duration = Duration::from_secs(30);
/// The most marks kept, so the folder never grows without bound.
const MAX_MARKS: usize = 256;
/// The folder the marks live in, inside muster's runtime folder.
const MARKS_DIR: &str = "session-context";
/// Why a marks folder is refused.
const NOT_PRIVATE: &str = "the session marks folder is not private to this user";

/// The session marks in one folder.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct SessionMarks {
    /// The folder, private to this user.
    dir: PathBuf,
}

impl SessionMarks {
    /// The marks in muster's private runtime folder for this user.
    pub fn in_runtime_dir() -> Self {
        Self::builder().dir(socket_dir().join(MARKS_DIR)).build()
    }

    /// Whether `input`'s session and source start here for the first time
    /// within [`MARK_TTL`] of `now`, leaving the mark that says so. Anything
    /// that keeps the mark from being read or left counts as a first start,
    /// so the context is never held back by a failure.
    pub fn first(&self, input: &HookInput, now: SystemTime) -> bool {
        if self.ready().is_err() {
            return true;
        }
        self.prune(now);
        let mark = self.dir.join(hash_key(&input.key()));
        match create_new(&mark, now) {
            Ok(()) => true,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                if fresh(&mark, now) {
                    return false;
                }
                let _ = fs::remove_file(&mark);
                create_new(&mark, now)
                    .map_or_else(|error| error.kind() != ErrorKind::AlreadyExists, |()| true)
            },
            Err(_) => true,
        }
    }

    /// Makes the folder, and the runtime folder above it, owner-only, and
    /// refuses either when another user could reach it.
    ///
    /// # Errors
    /// Returns the I/O error of making or checking them.
    fn ready(&self) -> io::Result<()> {
        if let Some(runtime) = self.dir.parent() {
            prepare_socket_dir(runtime)?;
        }
        create_private_dir_all(&self.dir)?;
        if is_owned_private_dir(&self.dir)? {
            Ok(())
        } else {
            Err(io::Error::new(ErrorKind::PermissionDenied, NOT_PRIVATE))
        }
    }

    /// Removes every mark older than [`MARK_TTL`] at `now`, then the oldest
    /// past [`MAX_MARKS`], leaving room for one more.
    fn prune(&self, now: SystemTime) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut kept: Vec<(SystemTime, PathBuf)> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            match modified(&path) {
                Some(at) if !expired(at, now) => kept.push((at, path)),
                _ => {
                    let _ = fs::remove_file(&path);
                },
            }
        }
        kept.sort();
        let excess = (kept.len() + 1).saturating_sub(MAX_MARKS);
        for (_, path) in kept.into_iter().take(excess) {
            let _ = fs::remove_file(path);
        }
    }
}

/// Creates the empty mark at `path`, left at `now`, failing when it exists.
///
/// # Errors
/// Returns the I/O error, of kind [`ErrorKind::AlreadyExists`] when another
/// run left it first.
fn create_new(path: &Path, now: SystemTime) -> io::Result<()> {
    let mark: File = OpenOptions::new().write(true).create_new(true).open(path)?;
    // A mark that cannot take the time still marks the start, by its own.
    let _ = mark.set_modified(now);
    Ok(())
}

/// When the mark at `path` was left.
fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

/// Whether a mark left `at` has run out at `now`.
fn expired(at: SystemTime, now: SystemTime) -> bool {
    now.duration_since(at).is_ok_and(|age| age >= MARK_TTL)
}

/// Whether the mark at `path` still holds at `now`.
fn fresh(path: &Path, now: SystemTime) -> bool {
    modified(path).is_some_and(|at| !expired(at, now))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    /// Marks in a fresh private scratch folder.
    fn scratch_marks() -> (SessionMarks, TempDir) {
        let scratch = TempDir::new().unwrap();
        let marks = SessionMarks::builder()
            .dir(scratch.path().join("run").join(MARKS_DIR))
            .build();
        (marks, scratch)
    }

    /// The input of `session` starting as `source`.
    fn input(session: &str, source: &str) -> HookInput {
        HookInput::builder()
            .session_id(session.to_owned())
            .source(Some(source.to_owned()))
            .build()
    }

    /// A session's start prints once, another source or session again, and
    /// once the mark runs out the same start prints again.
    #[test]
    fn a_start_prints_once_until_its_mark_runs_out() {
        let (marks, _scratch) = scratch_marks();
        let now = SystemTime::now();

        assert!(marks.first(&input("a", "startup"), now));
        assert!(!marks.first(&input("a", "startup"), now));
        assert!(marks.first(&input("a", "compact"), now));
        assert!(marks.first(&input("b", "startup"), now));

        let later = now + MARK_TTL + MARK_TTL;
        assert!(marks.first(&input("a", "startup"), later));
        assert!(!marks.first(&input("a", "startup"), later));
    }

    /// The folder never holds more than [`MAX_MARKS`], and run-out marks
    /// go.
    #[test]
    fn the_marks_are_bounded() {
        let (marks, _scratch) = scratch_marks();
        let now = SystemTime::now();
        for session in 0..MAX_MARKS + MAX_MARKS / 2 {
            marks.first(&input(&session.to_string(), "startup"), now);
        }

        assert!(fs::read_dir(marks.dir()).unwrap().count() <= MAX_MARKS);

        marks.first(&input("last", "startup"), now + MARK_TTL + MARK_TTL);
        assert_eq!(fs::read_dir(marks.dir()).unwrap().count(), 1);
    }

    /// A folder muster cannot use never holds the context back.
    #[test]
    fn an_unusable_folder_counts_as_a_first_start() {
        let scratch = TempDir::new().unwrap();
        let blocked = scratch.path().join("file");
        fs::write(&blocked, "").unwrap();
        let marks = SessionMarks::builder().dir(blocked.join(MARKS_DIR)).build();

        assert!(marks.first(&input("a", "startup"), SystemTime::now()));
        assert!(marks.first(&input("a", "startup"), SystemTime::now()));
    }
}

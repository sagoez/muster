//! Stand-in programs a test writes as shell scripts, ready to run the moment
//! they are written.

use std::{
    fs::{self, Permissions},
    io::ErrorKind,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
    thread,
    time::Instant,
};

use super::test_wait::{POLL, WAIT};

/// The argument a stand-in answers by exiting at once, doing nothing else.
const SETTLE_ARG: &str = "__muster_test_settle";
/// Mode of a stand-in: its owner may run it.
const SCRIPT_MODE: u32 = 0o700;

/// Writes the shell script `body` to `path` as a stand-in program and waits
/// until it can be run. Linux refuses to run a file another process holds
/// open for writing, and a fork any test thread makes while the script is
/// being written holds it until that child runs its own program; running
/// the script once with a settling argument, until that stops failing with
/// `ExecutableFileBusy`, means the code under test never sees that refusal.
pub fn runnable_script(path: &Path, body: &str) {
    fs::write(
        path,
        format!("#!/bin/sh\n[ \"$1\" = {SETTLE_ARG} ] && exit 0\n{body}"),
    )
    .unwrap();
    fs::set_permissions(path, Permissions::from_mode(SCRIPT_MODE)).unwrap();
    let started = Instant::now();
    loop {
        match Command::new(path).arg(SETTLE_ARG).status() {
            Ok(status) => {
                assert!(status.success(), "{} does not settle", path.display());
                return;
            },
            Err(error) if error.kind() == ErrorKind::ExecutableFileBusy => {
                assert!(started.elapsed() < WAIT, "{} stays busy", path.display());
                thread::sleep(POLL);
            },
            Err(error) => panic!("{} does not run: {error}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::test_scratch::short_scratch;

    /// A stand-in runs at once, settling without doing anything, and does
    /// its work on any other call.
    #[test]
    fn a_stand_in_settles_quietly_and_then_runs() {
        let scratch = short_scratch("ts-");
        let (script, ran) = (scratch.path().join("tool"), scratch.path().join("ran"));
        runnable_script(&script, &format!("touch '{}'\n", ran.display()));
        assert!(!ran.exists());

        assert!(Command::new(&script).status().unwrap().success());
        assert!(ran.exists());
    }
}

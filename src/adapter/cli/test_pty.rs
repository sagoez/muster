//! A pseudo-terminal standing in for the person's terminal in attach tests:
//! the session draws on and reads from its slave side, and the test types on
//! and reads from its master side, so no test touches a real terminal.

use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{fd::OwnedFd, unix::fs::OpenOptionsExt},
    sync::{Arc, Mutex, PoisonError},
    thread,
    time::{Duration, Instant},
};

use libc::O_NOCTTY;
use rustix::{
    pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt},
    termios::{Termios, Winsize, tcgetattr, tcsetwinsize},
};

use super::host_terminal::HostTerminal;

/// Bound on every wait; reaching it fails the test.
pub const WAIT: Duration = Duration::from_secs(10);
/// Interval between checks of a polled condition.
pub const POLL: Duration = Duration::from_millis(10);
/// Bytes read from the master at a time.
const READ_CHUNK: usize = 4096;

/// Polls `condition` until it holds, failing the test at [`WAIT`].
pub fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(started.elapsed() < WAIT, "timed out waiting for {what}");
        thread::sleep(POLL);
    }
}

/// Whether `haystack` contains `needle`.
pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// One pseudo-terminal, with everything drawn on it collected.
pub struct TestPty {
    /// The side a test types on and reads from.
    master: File,
    /// The side the session takes over.
    slave: OwnedFd,
    /// Everything drawn on the slave, collected from the master.
    drawn: Arc<Mutex<Vec<u8>>>,
}

impl TestPty {
    /// Opens a pseudo-terminal of `rows` by `cols`.
    pub fn open(rows: u16, cols: u16) -> Self {
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
        grantpt(&master).unwrap();
        unlockpt(&master).unwrap();
        let path = ptsname(&master, Vec::new()).unwrap();
        let slave: OwnedFd = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(O_NOCTTY)
            .open(path.to_str().unwrap())
            .unwrap()
            .into();
        let pty = Self {
            master: File::from(master),
            slave,
            drawn: Arc::new(Mutex::new(Vec::new())),
        };
        pty.resize(rows, cols);
        let mut reader = pty.master.try_clone().unwrap();
        let drawn = Arc::clone(&pty.drawn);
        thread::spawn(move || {
            let mut chunk = [0; READ_CHUNK];
            while let Ok(read) = reader.read(&mut chunk)
                && read > 0
            {
                drawn
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .extend_from_slice(&chunk[..read]);
            }
        });
        pty
    }

    /// The slave side as the terminal a session takes over.
    pub fn terminal(&self) -> HostTerminal {
        HostTerminal::new(
            self.slave.try_clone().unwrap(),
            self.slave.try_clone().unwrap(),
        )
    }

    /// Types `keys` as a person would.
    pub fn type_keys(&self, keys: &[u8]) {
        (&self.master).write_all(keys).unwrap();
    }

    /// Everything drawn so far.
    pub fn drawn(&self) -> Vec<u8> {
        self.drawn
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Waits until `needle` has been drawn.
    pub fn wait_for(&self, needle: &[u8]) {
        wait_until(
            &format!("{:?} drawn", String::from_utf8_lossy(needle)),
            || contains(&self.drawn(), needle),
        );
    }

    /// Resizes the terminal, as a person dragging its window does.
    pub fn resize(&self, rows: u16, cols: u16) {
        tcsetwinsize(&self.master, Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        })
        .unwrap();
    }

    /// The terminal's current settings.
    pub fn settings(&self) -> Termios {
        tcgetattr(&self.slave).unwrap()
    }
}

/// Whether two terminal settings agree on everything raw mode changes.
pub fn same_settings(left: &Termios, right: &Termios) -> bool {
    left.input_modes == right.input_modes
        && left.output_modes == right.output_modes
        && left.control_modes == right.control_modes
        && left.local_modes == right.local_modes
}

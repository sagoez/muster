use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::OwnedFd,
        unix::{fs::OpenOptionsExt, process::CommandExt},
    },
    path::Path,
    process::{Child, Stdio},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Instant,
};

use libc::O_NOCTTY;
use rustix::{
    io::ioctl_fionread,
    process::{ioctl_tiocsctty, setsid},
    pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt},
    stdio::stdin,
    termios::{LocalModes, Winsize, tcgetattr, tcsetwinsize},
};

use super::{
    scratch::Scratch,
    wait::{POLL, WAIT, wait_until},
};

/// Rows of the pseudo-terminal a prompting command runs on.
const PTY_ROWS: u16 = 40;
/// Columns of that pseudo-terminal.
const PTY_COLS: u16 = 120;
/// Bytes read from a pseudo-terminal's master at a time.
const PTY_CHUNK: usize = 4096;
/// A terminal type that draws color, so the interactive layout is chosen.
const COLOR_TERM: &str = "xterm-256color";
/// The Ctrl-C key.
const CTRL_C: &[u8] = b"\x03";

/// A pseudo-terminal a command runs on as a person's terminal: the test
/// types on its master side and collects everything drawn there, so no test
/// touches a real terminal.
pub struct Pty {
    /// The side the test types on and reads from.
    master: File,
    /// The side the command runs on.
    slave: OwnedFd,
    /// Everything drawn on the slave, collected from the master.
    drawn: Arc<Mutex<Vec<u8>>>,
    /// Clear while the reader stores what it just read.
    idle: Arc<AtomicBool>,
}

impl Pty {
    /// Opens a pseudo-terminal and starts collecting what is drawn on it.
    pub fn open() -> Self {
        Self::open_sized(PTY_ROWS, PTY_COLS)
    }

    /// Opens a `rows` by `cols` pseudo-terminal and starts collecting what is
    /// drawn on it.
    pub fn open_sized(rows: u16, cols: u16) -> Self {
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
        tcsetwinsize(&master, Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        })
        .unwrap();
        let pty = Self {
            master: File::from(master),
            slave,
            drawn: Arc::new(Mutex::new(Vec::new())),
            idle: Arc::new(AtomicBool::new(true)),
        };
        let mut reader = pty.master.try_clone().unwrap();
        let drawn = Arc::clone(&pty.drawn);
        let idle = Arc::clone(&pty.idle);
        thread::spawn(move || {
            let mut chunk = [0; PTY_CHUNK];
            while let Ok(read) = reader.read(&mut chunk)
                && read > 0
            {
                idle.store(false, Ordering::SeqCst);
                drawn
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .extend_from_slice(&chunk[..read]);
                idle.store(true, Ordering::SeqCst);
            }
        });
        pty
    }

    /// Runs `muster <args>` over `scratch` from `folder`, with this terminal
    /// as its input, output and error.
    pub fn spawn(&self, scratch: &mut Scratch, folder: &Path, args: &[&str]) -> Child {
        let stdio = || Stdio::from(self.slave.try_clone().unwrap());
        let child = scratch
            .command(args)
            .current_dir(folder)
            .env("TERM", COLOR_TERM)
            .env_remove("NO_COLOR")
            .stdin(stdio())
            .stdout(stdio())
            .stderr(stdio())
            .spawn()
            .unwrap();
        scratch.keep_child(child.id());
        child
    }

    /// Types `keys` as a person would, once the command reads keys raw, so a
    /// Ctrl-C reaches it as a key rather than as a signal.
    pub fn type_keys(&self, keys: &[u8]) {
        wait_until("the terminal to read keys raw", || {
            !tcgetattr(&self.slave)
                .unwrap()
                .local_modes
                .contains(LocalModes::ICANON)
        });
        (&self.master).write_all(keys).unwrap();
    }

    /// Runs `muster <args>` like [`Pty::spawn`], but as the leader of its
    /// own session with this terminal as its controlling one, so a Ctrl-C
    /// typed while it is not reading keys reaches it as `SIGINT`.
    pub fn spawn_controlling(&self, scratch: &mut Scratch, folder: &Path, args: &[&str]) -> Child {
        let stdio = || Stdio::from(self.slave.try_clone().unwrap());
        let mut command = scratch.command(args);
        command
            .current_dir(folder)
            .env("TERM", COLOR_TERM)
            .env_remove("NO_COLOR")
            .stdin(stdio())
            .stdout(stdio())
            .stderr(stdio());
        // SAFETY: the hook runs between fork and exec and only makes the
        // `setsid` and `TIOCSCTTY` system calls, which are async-signal-safe
        // and touch only the child's own session and its standard input.
        unsafe {
            command.pre_exec(|| {
                setsid()?;
                ioctl_tiocsctty(stdin())?;
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        scratch.keep_child(child.id());
        child
    }

    /// Types Ctrl-C at once, whether or not the command reads keys raw.
    pub fn interrupt(&self) {
        (&self.master).write_all(CTRL_C).unwrap();
    }

    /// Everything drawn so far, escape sequences included.
    pub fn raw(&self) -> String {
        let bytes = self
            .drawn
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Everything drawn so far, as text with the styling sequences removed.
    pub fn drawn(&self) -> String {
        unstyled(&self.raw())
    }

    /// Waits until `needle` has been drawn `times` times in all.
    pub fn wait_for(&self, needle: &str, times: usize) {
        wait_until(&format!("{needle:?} drawn {times} times"), || {
            self.drawn().matches(needle).count() >= times
        });
    }

    /// Waits for `child` to exit, then for everything it drew to be
    /// collected, and returns its exit code.
    pub fn exit_code(&self, child: &mut Child) -> Option<i32> {
        let started = Instant::now();
        let code = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status.code();
            }
            assert!(started.elapsed() < WAIT, "the command did not exit");
            thread::sleep(POLL);
        };
        self.drained();
        code
    }

    /// Waits until nothing drawn waits on the master unread and the reader
    /// has stored what it read, on two checks in a row, so a test reading
    /// what an exited command drew sees all of it.
    fn drained(&self) {
        let quiet =
            || ioctl_fionread(&self.master).unwrap_or(0) == 0 && self.idle.load(Ordering::SeqCst);
        wait_until("everything drawn to be collected", || {
            if !quiet() {
                return false;
            }
            thread::sleep(POLL);
            quiet()
        });
    }
}

/// `text` with its CSI and OSC escape sequences removed.
fn unstyled(text: &str) -> String {
    let mut plain = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            plain.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            },
            Some(']') => {
                for c in chars.by_ref() {
                    if c == '\x07' {
                        break;
                    }
                }
            },
            _ => {},
        }
    }
    plain
}

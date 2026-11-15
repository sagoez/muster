use std::{
    io::{BufRead, BufReader},
    os::unix::process::CommandExt,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::Instant,
};

use crossbeam_channel::{Receiver, unbounded};
use muster_web::{EventReader, HttpReply, get};
use rustix::process::{Pid, Signal, kill_process, kill_process_group};

use crate::common::{
    scratch::Scratch,
    wait::{POLL, WAIT},
};

/// What the view's address starts with, before its port.
const URL_START: &str = "http://127.0.0.1:";
/// What follows the port in the view's address, before its token.
const TOKEN_START: &str = "/v/";
/// What ends the view's path, after its token.
const URL_END: &str = "/";
/// What starts the fragment naming the floor the page opens on.
const FRAGMENT_START: char = '#';
/// The live stream's path below the token's.
const EVENTS_PATH: &str = "events";
/// The subcommand that serves the view, and the flag that keeps it from
/// opening a browser.
const VIEW: &str = "view";
const NO_OPEN: &str = "--no-open";
/// The global flag naming the project.
const PROJECT_FLAG: &str = "-p";
/// The variable naming the browser the view opens.
const BROWSER_ENV: &str = "BROWSER";
/// The process group a spawned view leads: one of its own, as a shell's
/// job control gives it.
const OWN_PROCESS_GROUP: i32 = 0;

/// A `muster view` run over a scratch, its address read from the first line
/// it printed; dropping it kills the view.
pub struct ViewSession {
    /// The view.
    child: Child,
    /// What it printed after its address, a line at a time.
    lines: Receiver<String>,
    /// The address it printed.
    url: String,
    /// The port in that address.
    port: u16,
    /// The token in that address.
    token: String,
    /// The fragment that address ends with, empty for none.
    fragment: String,
}

impl ViewSession {
    /// Starts `muster view --no-open` with `args` over `scratch` and reads
    /// its address.
    pub fn start(scratch: &mut Scratch, args: &[&str]) -> Self {
        let mut all = vec![VIEW, NO_OPEN];
        all.extend_from_slice(args);
        Self::launch(scratch.command(&all), scratch)
    }

    /// Starts `muster -p <project> view --no-open` over `scratch` and reads
    /// its address.
    pub fn for_project(scratch: &mut Scratch, project: &str) -> Self {
        Self::launch(
            scratch.command(&[PROJECT_FLAG, project, VIEW, NO_OPEN]),
            scratch,
        )
    }

    /// Starts `muster view` over `scratch`, opening its page in `browser`,
    /// as a shell job: in a process group of its own, which
    /// [`Self::signal_group`] signals as a terminal's Ctrl-C does.
    pub fn opening_in(scratch: &mut Scratch, browser: &Path) -> Self {
        let mut command = scratch.command(&[VIEW]);
        command
            .env(BROWSER_ENV, browser)
            .process_group(OWN_PROCESS_GROUP);
        Self::launch(command, scratch)
    }

    /// Spawns `command`, keeps it for `scratch` to kill, and reads the
    /// address it prints first.
    fn launch(mut command: Command, scratch: &mut Scratch) -> Self {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        scratch.keep_child(child.id());
        let out = child.stdout.take().unwrap();
        let (sender, lines) = unbounded();
        thread::spawn(move || {
            for line in BufReader::new(out).lines() {
                let Ok(line) = line else { return };
                if sender.send(line).is_err() {
                    return;
                }
            }
        });
        let url = lines
            .recv_timeout(WAIT)
            .unwrap_or_else(|_| panic!("muster view printed no address"));
        let (address, fragment) = url
            .split_once(FRAGMENT_START)
            .map_or((url.as_str(), String::new()), |(address, fragment)| {
                (address, format!("{FRAGMENT_START}{fragment}"))
            });
        let (port, token) = address
            .strip_prefix(URL_START)
            .and_then(|rest| rest.split_once(TOKEN_START))
            .and_then(|(port, rest)| Some((port, rest.strip_suffix(URL_END)?)))
            .unwrap_or_else(|| panic!("not the view's address: {url:?}"));
        let port = port.parse().unwrap();
        let token = token.to_string();
        Self {
            child,
            lines,
            url,
            port,
            token,
            fragment,
        }
    }

    /// The address the view printed first.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The fragment that address ends with, naming the floor the page
    /// opens on; empty for none.
    pub fn fragment(&self) -> &str {
        &self.fragment
    }

    /// The port the view listens on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The view's token.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The view's answer to a GET of `below` its token's path.
    pub fn get(&self, below: &str) -> HttpReply {
        get(self.port, &self.below_token(below), &[])
    }

    /// The view's live stream, its head read.
    pub fn events(&self) -> EventReader {
        EventReader::open(self.port, &self.below_token(EVENTS_PATH))
    }

    /// The path of `below` under the view's token.
    fn below_token(&self, below: &str) -> String {
        format!("{TOKEN_START}{}{URL_END}{below}", self.token)
    }

    /// The next line the view printed after its address.
    pub fn next_line(&self) -> String {
        self.lines
            .recv_timeout(WAIT)
            .unwrap_or_else(|_| panic!("muster view printed no further line"))
    }

    /// Whether the view still runs.
    pub fn running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    /// Sends the view `signal` and waits for it to end.
    pub fn signal(&mut self, signal: Signal) -> ExitStatus {
        kill_process(self.pid(), signal).unwrap();
        self.wait_out(signal)
    }

    /// Sends `signal` to the view's whole process group, as a terminal does
    /// to its foreground job, and waits for the view to end.
    pub fn signal_group(&mut self, signal: Signal) -> ExitStatus {
        kill_process_group(self.pid(), signal).unwrap();
        self.wait_out(signal)
    }

    /// The view's pid.
    fn pid(&self) -> Pid {
        Pid::from_raw(i32::try_from(self.child.id()).unwrap()).unwrap()
    }

    /// Waits for the view to end after `signal`, failing past [`WAIT`].
    fn wait_out(&mut self, signal: Signal) -> ExitStatus {
        let sent = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(sent.elapsed() < WAIT, "muster view outlived {signal:?}");
            thread::sleep(POLL);
        }
    }
}

impl Drop for ViewSession {
    /// Kills the view and reaps it.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

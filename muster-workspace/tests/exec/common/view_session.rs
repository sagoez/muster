use std::{
    io::{BufRead, BufReader},
    process::{Child, ExitStatus, Stdio},
    thread,
    time::Instant,
};

use crossbeam_channel::{Receiver, unbounded};
use rustix::process::{Pid, Signal, kill_process};

use super::{
    scratch::Scratch,
    wait::{POLL, WAIT},
};

/// What the view's address starts with, before its port.
const URL_START: &str = "http://127.0.0.1:";
/// What follows the port in the view's address, before its token.
const TOKEN_START: &str = "/v/";
/// What ends the view's address, after its token.
const URL_END: &str = "/";

/// A `muster view --no-open` run over a scratch, its address read from the
/// first line it printed; dropping it kills the view.
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
}

impl ViewSession {
    /// Starts `muster view --no-open` with `args` over `scratch` and reads
    /// its address.
    pub fn start(scratch: &mut Scratch, args: &[&str]) -> Self {
        let mut all = vec!["view", "--no-open"];
        all.extend_from_slice(args);
        let mut child = scratch
            .command(&all)
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
        let (port, token) = url
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
        }
    }

    /// The address the view printed first.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The port the view listens on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The view's token.
    pub fn token(&self) -> &str {
        &self.token
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

    /// Sends the view the Ctrl-C a terminal would and waits for it to end.
    pub fn interrupt(&mut self) -> ExitStatus {
        let pid = Pid::from_raw(i32::try_from(self.child.id()).unwrap()).unwrap();
        kill_process(pid, Signal::INT).unwrap();
        let sent = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(sent.elapsed() < WAIT, "muster view outlived its Ctrl-C");
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

use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;

use super::{
    scratch::Scratch,
    shell_session::on_path,
    wait::{POLL, WAIT, wait_until},
};

/// The variable that turns the real-herdr tests on.
const GATE_ENV: &str = "MUSTER_HERDR_TEST";
/// The value that turns them on.
const GATE_ON: &str = "1";
/// The herdr binary, found on `PATH`.
const HERDR: &str = "herdr";
/// The shell herdr's panes run.
const PANE_SHELL: &str = "bash";
/// The terminal type herdr's panes report.
const PANE_TERM: &str = "xterm-256color";
/// herdr's socket, inside the scratch.
const SOCKET: &str = "herdr.sock";
/// The home folder herdr and its panes see, inside the scratch.
const HOME: &str = "herdr-home";
/// herdr's config file, under the scratch's config root.
const CONFIG: &str = "config/herdr/config.toml";
/// The variable naming herdr's socket.
const SOCKET_ENV: &str = "HERDR_SOCKET_PATH";
/// The variable naming herdr's config file.
const CONFIG_ENV: &str = "HERDR_CONFIG_PATH";
/// The variable naming the pane a command runs in.
const PANE_ENV: &str = "HERDR_PANE_ID";
/// The variable naming the herdr binary.
const BIN_ENV: &str = "HERDR_BIN_PATH";
/// The variable naming the home folder.
const HOME_ENV: &str = "HOME";
/// The variable naming the shell a pane runs.
const SHELL_ENV: &str = "SHELL";
/// The variable naming the terminal type.
const TERM_ENV: &str = "TERM";
/// How long a `pane read` reaches back, in lines.
const READ_LINES: &str = "200";
/// How long the server is given to stop on its own before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(10);

/// A real herdr server of a test's own, its socket, config, state and home
/// inside the test's scratch, stopped and killed on drop. Started only when
/// `MUSTER_HERDR_TEST=1` and herdr is installed.
pub struct HerdrServer {
    /// The herdr binary.
    program: PathBuf,
    /// The variables every herdr command here runs with, or removes.
    env: Vec<(OsString, Option<OsString>)>,
    /// The folder every herdr command here runs from.
    dir: PathBuf,
    /// The home folder herdr's panes start their shells in.
    home: PathBuf,
    /// The server.
    server: Child,
}

impl HerdrServer {
    /// Starts a herdr server over `scratch`, once its socket answers from
    /// inside the scratch; `None`, with a notice naming `test`, when the
    /// real-herdr tests are off, herdr is not installed, or the socket the
    /// server reached lies anywhere else.
    pub fn start(scratch: &Scratch, test: &str) -> Option<Self> {
        let gate = env::var_os(GATE_ENV);
        let program = on_path(HERDR);
        let missing = match (gate.as_deref() == Some(OsStr::new(GATE_ON)), &program) {
            (true, Some(_)) => None,
            (false, Some(_)) => Some(format!("{GATE_ENV}={GATE_ON} is not set")),
            (true, None) => Some(format!("{HERDR} is not on PATH")),
            (false, None) => Some(format!(
                "{GATE_ENV}={GATE_ON} is not set and {HERDR} is not on PATH"
            )),
        };
        if let Some(missing) = missing {
            eprintln!(
                "herdr end-to-end skipped: set {GATE_ENV}={GATE_ON} with herdr installed \
                 ({missing}): {test}"
            );
            return None;
        }
        let program = program?;
        let root = scratch.path();
        let socket = root.join(SOCKET);
        let home = root.join(HOME);
        fs::create_dir_all(&home).unwrap();
        let mut base = scratch.isolated(&program);
        base.env(HOME_ENV, &home)
            .env(SHELL_ENV, on_path(PANE_SHELL).expect("bash is installed"))
            .env(TERM_ENV, PANE_TERM)
            .env(SOCKET_ENV, &socket)
            .env(CONFIG_ENV, root.join(CONFIG));
        let env = base
            .get_envs()
            .map(|(name, value)| (name.to_os_string(), value.map(OsStr::to_os_string)))
            .collect();
        let server = base
            .arg("server")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let herdr = Self {
            program,
            env,
            dir: scratch.root().to_path_buf(),
            home,
            server,
        };
        wait_until("the herdr server to answer", || {
            herdr
                .command(&["status", "--json"])
                .output()
                .is_ok_and(|status| {
                    serde_json::from_slice::<Value>(&status.stdout)
                        .is_ok_and(|status| status["server"]["running"] == true)
                })
        });
        let status = herdr.json(&["status", "--json"]);
        let reached = status["server"]["socket"].as_str().map(PathBuf::from);
        if reached.as_deref() != Some(socket.as_path()) {
            eprintln!(
                "herdr end-to-end skipped: the server's socket {reached:?} is not {}: {test}",
                socket.display()
            );
            return None;
        }
        Some(herdr)
    }

    /// The home folder of the shells herdr's panes run, whose rc files they
    /// read.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// `herdr <args>` against this server.
    pub fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.program);
        for (name, value) in &self.env {
            match value {
                Some(value) => command.env(name, value),
                None => command.env_remove(name),
            };
        }
        command.current_dir(&self.dir).args(args);
        command
    }

    /// Runs `command` as if in `pane` of this server: herdr's own variables
    /// for the pane, its binary and its socket.
    pub fn place<'a>(&self, command: &'a mut Command, pane: &str) -> &'a mut Command {
        for (name, value) in &self.env {
            if let Some(value) = value
                && [SOCKET_ENV, CONFIG_ENV]
                    .map(OsStr::new)
                    .contains(&name.as_os_str())
            {
                command.env(name, value);
            }
        }
        command.env(PANE_ENV, pane).env(BIN_ENV, &self.program)
    }

    /// Runs `herdr <args>`, failing the test unless it succeeds.
    pub fn run(&self, args: &[&str]) -> Output {
        let output = self.command(args).output().unwrap();
        assert!(
            output.status.success(),
            "herdr {args:?}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    /// Runs `herdr <args>` and parses what it printed.
    pub fn json(&self, args: &[&str]) -> Value {
        serde_json::from_slice(&self.run(args).stdout).unwrap()
    }

    /// Opens a workspace whose root pane's shell starts in `cwd`, and
    /// returns that pane.
    pub fn workspace(&self, cwd: &Path) -> String {
        let created = self.json(&["workspace", "create", "--cwd", cwd.to_str().unwrap()]);
        created["result"]["root_pane"]["pane_id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// Every pane of the server.
    pub fn panes(&self) -> Vec<Value> {
        self.json(&["pane", "list"])["result"]["panes"]
            .as_array()
            .unwrap()
            .clone()
    }

    /// The listing of `pane`, if the server still has it.
    pub fn pane(&self, pane: &str) -> Option<Value> {
        self.panes()
            .into_iter()
            .find(|listed| listed["pane_id"] == pane)
    }

    /// Waits until `pane` shows `text`.
    pub fn wait_output(&self, pane: &str, text: &str) {
        let bound = WAIT.as_millis().to_string();
        let output = self
            .command(&[
                "pane",
                "wait-output",
                pane,
                "--match",
                text,
                "--timeout",
                &bound,
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{pane} never showed {text:?}: {}\n{}",
            String::from_utf8_lossy(&output.stderr),
            self.read(pane)
        );
    }

    /// What `pane` shows, its recent lines joined where they wrapped.
    pub fn read(&self, pane: &str) -> String {
        let output = self
            .command(&[
                "pane",
                "read",
                pane,
                "--source",
                "recent-unwrapped",
                "--lines",
                READ_LINES,
            ])
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Types `text` into `pane` as it is, without an Enter.
    pub fn send_text(&self, pane: &str, text: &str) {
        self.run(&["pane", "send-text", pane, text]);
    }

    /// Types `line` and an Enter into `pane`.
    pub fn type_line(&self, pane: &str, line: &str) {
        self.run(&["pane", "run", pane, line]);
    }

    /// Waits until the server no longer lists `pane`.
    pub fn wait_closed(&self, pane: &str) {
        wait_until(&format!("pane {pane} to close"), || {
            self.pane(pane).is_none()
        });
    }
}

impl Drop for HerdrServer {
    /// Asks the server to stop, which ends every pane's shell, and kills it,
    /// and the stop request, when it has not stopped within [`STOP_GRACE`].
    fn drop(&mut self) {
        let mut stop = self
            .command(&["server", "stop"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok();
        let started = Instant::now();
        while started.elapsed() < STOP_GRACE && !matches!(self.server.try_wait(), Ok(Some(_))) {
            thread::sleep(POLL);
        }
        for child in stop.iter_mut().chain([&mut self.server]) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

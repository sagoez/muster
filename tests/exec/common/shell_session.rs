use std::{
    cell::Cell,
    env,
    path::{Path, PathBuf},
    process::Child,
    thread,
    time::{Duration, Instant},
};

use muster::{
    adapter::shell::{HookShell, SHELL_REPORT_SUBCOMMAND, ShellFiles},
    domain::{occupancy::ShellRecord, port::ShellRegistry, value::Pid},
};
use rustix::process::{Pid as RawPid, Signal, kill_process};

use super::{
    processes::processes_under,
    pty::Pty,
    scratch::{BINARY, Scratch},
    wait::{POLL, wait_until},
};

/// The variable the hook exports, cleared so no outer hooked shell leaks in.
const MUSTER_SHELL_ENV: &str = "MUSTER_SHELL_PID";
/// What each marker command prints, before its number.
const MARKER_PREFIX: &str = "mark-";
/// What each marker command prints, after its number.
const MARKER_SUFFIX: &str = "-done";
/// What the status command prints, before the last exit status.
const STATUS_PREFIX: &str = "status-";
/// The Enter key.
const ENTER: &str = "\r";
/// What a probe runs to learn whether a shell runs.
const PROBE_SCRIPT: &str = "exit 0";
/// Checks in a row that must find no report running before the hook's
/// reports count as finished.
const QUIET_CHECKS: usize = 3;

/// A person's interactive shell running muster's hook on a pseudo-terminal,
/// over a scratch's XDG roots; dropping it kills the shell and every report
/// its hook left running, before the scratch's own teardown.
pub struct ShellSession {
    /// Which shell it is.
    shell: HookShell,
    /// The program it runs.
    program: PathBuf,
    /// The terminal it runs on.
    pty: Pty,
    /// The shell process.
    child: Child,
    /// The scratch root, which every report's environment names.
    root: PathBuf,
    /// muster's state directory, holding the shell files.
    state_dir: PathBuf,
    /// The number of the next marker command.
    markers: Cell<u64>,
}

impl ShellSession {
    /// Starts `program` as `shell` with no rc file, in the scratch root,
    /// loads muster's hook as its init line says, and waits until the hook
    /// has reported the shell.
    pub fn start(scratch: &mut Scratch, shell: HookShell, program: &Path) -> Self {
        let mut command = scratch.isolated(program);
        command
            .args(no_rc_args(shell))
            .env("HOME", scratch.root())
            .env_remove(MUSTER_SHELL_ENV);
        let pty = Pty::open();
        let child = pty.spawn_leading(scratch, command);
        let session = Self {
            shell,
            program: program.to_path_buf(),
            pty,
            child,
            root: scratch.root().to_path_buf(),
            state_dir: scratch.state_dir(),
            markers: Cell::new(0),
        };
        session.load_hook();
        wait_until("the hook to report the shell", || {
            session.record().is_some()
        });
        session
    }

    /// Types the line that loads muster's hook, as a person's rc file
    /// holds it, and waits for the shell to settle.
    pub fn load_hook(&self) {
        let binary = shlex::try_quote(BINARY).unwrap();
        let line = match self.shell {
            HookShell::Bash | HookShell::Zsh => {
                format!("eval \"$({binary} shell-init {})\"", self.shell)
            },
            HookShell::Fish => format!("{binary} shell-init fish | source"),
        };
        self.run(&line);
    }

    /// Makes the hook run `binary` for its reports from the next prompt on,
    /// as a slow wrapper around muster would be.
    pub fn report_through(&self, binary: &Path) {
        let binary = shlex::try_quote(binary.to_str().unwrap()).unwrap();
        let line = match self.shell {
            HookShell::Bash | HookShell::Zsh => format!("__muster_bin={binary}"),
            HookShell::Fish => format!("set -g __muster_bin {binary}"),
        };
        self.run(&line);
    }

    /// Replaces the shell with a fresh one of the same program on the same
    /// pid, as `exec $SHELL` does, and loads the hook in it as its rc file
    /// would.
    pub fn exec_again(&self) {
        let program = shlex::try_quote(self.program.to_str().unwrap()).unwrap();
        self.run(&format!(
            "exec {program} {}",
            no_rc_args(self.shell).join(" ")
        ));
        self.load_hook();
    }

    /// Types `line` at the prompt, then waits until the shell has run it,
    /// passed one prompt and run a marker command after it.
    pub fn run(&self, line: &str) {
        self.type_line(line);
        self.settle();
    }

    /// Changes the shell's folder to `folder` and waits for the next prompt.
    pub fn cd(&self, folder: &Path) {
        let quoted = shlex::try_quote(folder.to_str().unwrap()).unwrap();
        self.run(&format!("cd {quoted}"));
    }

    /// Types `line` and Enter once the shell reads keys at a prompt, without
    /// waiting for it to run.
    pub fn type_line(&self, line: &str) {
        self.pty.type_keys(format!("{line}{ENTER}").as_bytes());
    }

    /// Runs one marker command and waits for what it prints, so every line
    /// typed before it has run and the prompt before it has passed its hook.
    pub fn settle(&self) {
        let _ = self.round_trip();
    }

    /// How long one marker command takes from the moment it is ready to be
    /// typed until its output is drawn: the prompt the shell passes before it
    /// reads the line, the hook included, plus running it.
    pub fn round_trip(&self) -> Duration {
        let number = self.markers.get() + 1;
        self.markers.set(number);
        let line = match self.shell {
            HookShell::Bash | HookShell::Zsh => {
                format!("echo {MARKER_PREFIX}$(( {number} )){MARKER_SUFFIX}")
            },
            HookShell::Fish => format!("echo {MARKER_PREFIX}(math {number}){MARKER_SUFFIX}"),
        };
        let started = Instant::now();
        self.type_line(&line);
        self.pty
            .wait_for(&format!("{MARKER_PREFIX}{number}{MARKER_SUFFIX}"), 1);
        started.elapsed()
    }

    /// Types the command that prints the last exit status, behind
    /// [`STATUS_PREFIX`], and returns what it prints for `status`.
    pub fn echo_status(&self, status: i32) -> String {
        let line = match self.shell {
            HookShell::Bash | HookShell::Zsh => format!("echo {STATUS_PREFIX}$?"),
            HookShell::Fish => format!("echo {STATUS_PREFIX}$status"),
        };
        self.type_line(&line);
        format!("{STATUS_PREFIX}{status}")
    }

    /// Waits until `needle` has been drawn.
    pub fn wait_drawn(&self, needle: &str) {
        self.pty.wait_for(needle, 1);
    }

    /// Everything drawn so far, as text with the styling removed.
    pub fn drawn(&self) -> String {
        self.pty.drawn()
    }

    /// The shell's pid, as its hook reports it.
    pub fn pid(&self) -> Pid {
        Pid::try_new(self.child.id()).unwrap()
    }

    /// The record muster keeps of this shell's last report, if any.
    pub fn record(&self) -> Option<ShellRecord> {
        let shells = ShellFiles::under(&self.state_dir);
        shells.record(&shells.identify(self.pid())?)
    }

    /// Waits until no report this scratch's hooks started is running, on
    /// [`QUIET_CHECKS`] checks in a row.
    pub fn wait_reports_done(&self) {
        let mut quiet = 0;
        wait_until("the hook's reports to finish", || {
            if processes_under(SHELL_REPORT_SUBCOMMAND, &self.root).is_empty() {
                quiet += 1;
            } else {
                quiet = 0;
            }
            thread::sleep(POLL);
            quiet >= QUIET_CHECKS
        });
    }

    /// Kills the shell at once, as closing its terminal window abruptly
    /// would, and reaps it.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for ShellSession {
    /// Kills the shell, then every report its hook left running, until none
    /// is left, so none starts a service after the scratch's teardown.
    fn drop(&mut self) {
        self.kill();
        let mut quiet = 0;
        while quiet < QUIET_CHECKS {
            let reports = processes_under(SHELL_REPORT_SUBCOMMAND, &self.root);
            quiet = if reports.is_empty() { quiet + 1 } else { 0 };
            for pid in reports {
                if let Some(pid) = RawPid::from_raw(pid as i32) {
                    let _ = kill_process(pid, Signal::KILL);
                }
            }
            thread::sleep(POLL);
        }
    }
}

/// The program for `shell` on `PATH`, when it runs over `scratch`'s XDG
/// roots: a missing shell, or a shim that refuses to run there, gives none.
pub fn runnable_shell(scratch: &Scratch, shell: HookShell) -> Option<PathBuf> {
    let program = on_path(&shell.to_string())?;
    scratch
        .isolated(&program)
        .args(["-c", PROBE_SCRIPT])
        .output()
        .is_ok_and(|output| output.status.success())
        .then_some(program)
}

/// The program `name` on `PATH`, if any.
pub fn on_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// The arguments that start `shell` interactive with no rc file.
pub fn no_rc_args(shell: HookShell) -> &'static [&'static str] {
    match shell {
        HookShell::Bash => &["--norc", "--noprofile", "-i"],
        HookShell::Zsh => &["-f", "-i"],
        HookShell::Fish => &["--no-config", "-i"],
    }
}

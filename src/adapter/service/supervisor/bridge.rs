use super::{entity::Supervisor, holder_screen::HolderScreen, screen_reply::ScreenReply};
use crate::{
    adapter::service::NoticeLevel,
    domain::{
        pty::PtyError,
        value::{PaneId, ProcessName},
    },
};

/// Prefix for a holder whose screen could not be read; the service's own copy
/// of the screen was answered instead.
const SCREEN_READ_FAILED_NOTICE: &str =
    "could not read this process's screen from its holder; showing the service's copy";

impl Supervisor {
    /// The visible screen of the process `name`, or `None` when the project
    /// has no such process. It never blocks: a screen only a holder can serve
    /// comes back as [`ScreenReply::ReadHolder`] for the caller to read.
    pub fn screen(&self, name: &ProcessName) -> Option<ScreenReply> {
        self.pane_named(name).map(|pane| self.pane_screen(pane))
    }

    /// The visible screen of the known process on `pane`. A live process
    /// whose handle names a holder is answered by that holder, so the read is
    /// handed to the caller; otherwise the pane's own terminal answers, and a
    /// process that has not started its terminal yet answers empty.
    fn pane_screen(&self, pane: PaneId) -> ScreenReply {
        let Some(target) = self.panes.get(&pane) else {
            return ScreenReply::Ready(String::new());
        };
        let fallback = target.parser.screen_text();
        let socket = target
            .handle
            .as_ref()
            .and_then(|handle| handle.holder_socket());
        match (socket, self.generations.get(&pane)) {
            (Some(socket), Some(generation)) => ScreenReply::ReadHolder(
                HolderScreen::builder()
                    .project(self.root().clone())
                    .pane(pane)
                    .generation(*generation)
                    .socket(socket.to_path_buf())
                    .fallback(fallback)
                    .build(),
            ),
            _ => ScreenReply::Ready(fallback),
        }
    }

    /// Answers a holder screen read the caller performed: the holder's screen,
    /// or the service's mirror when the read failed. The first failure of a
    /// streak raises a warning; a successful read ends the streak. A read that
    /// outlived its process (another project, or a newer spawn of the pane)
    /// is answered as it stands and leaves the streak and the notices alone.
    pub fn settle_holder_read(
        &mut self,
        screen: HolderScreen,
        read: Result<String, PtyError>,
    ) -> String {
        let pane = screen.pane();
        let current = screen.project() == self.root()
            && self.panes.contains_key(&pane)
            && self.generations.get(&pane) == Some(&screen.generation());
        if !current {
            return read.unwrap_or_else(|_| screen.fallback().clone());
        }
        match read {
            Ok(text) => {
                self.failing_screen_reads.remove(&pane);
                text
            },
            Err(error) => {
                if self.failing_screen_reads.insert(pane) {
                    self.notify(
                        NoticeLevel::Warning,
                        Some(pane),
                        format!("{SCREEN_READ_FAILED_NOTICE}: {error}"),
                    );
                }
                screen.fallback().clone()
            },
        }
    }

    /// Writes `data` to the terminal of the process `name`, as keyboard input
    /// would. Returns whether it reached a live terminal; it does not for a
    /// process that is not running or a name the project lacks.
    pub fn send_input(&mut self, name: &ProcessName, data: &[u8]) -> bool {
        self.pane_named(name)
            .and_then(|pane| self.panes.get_mut(&pane))
            .and_then(|target| target.handle.as_mut())
            .is_some_and(|handle| handle.write_input(data).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        os::unix::net::{UnixListener, UnixStream},
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
        thread,
    };

    use super::*;
    use crate::{
        adapter::{
            holder::{ClientMessage, HolderMessage},
            service::{LaunchEnvironment, supervisor::testing::*},
        },
        domain::{
            port::{OutputSink, ProcessHandle, ProcessRunner},
            process::RestartPolicy,
            pty::{ExitOutcome, ProcessOutput, PtySize, SpawnRequest},
            value::OutputCursor,
        },
    };

    const PANE: u64 = 0;
    /// The name of the process on [`PANE`].
    const NAME: &str = "p";
    /// A name no process has.
    const UNKNOWN: &str = "nobody";
    /// Short root for test sockets: macOS's long temp dir breaches `sun_path`.
    const SHORT_TEMP_ROOT: &str = "/tmp";
    /// Prefix of every test scratch directory.
    const SCRATCH_PREFIX: &str = "mb-";
    /// What the fake holder's screen shows.
    const HOLDER_SCREEN: &str = "drawn by the holder";
    /// What the service's own terminal of the same process shows.
    const SERVICE_SCREEN: &str = "drawn by the service";
    /// Rows and columns of the fake holder's screen.
    const HOLDER_SIDE: u16 = 24;
    /// A socket path nothing serves.
    const UNSERVED_SOCKET: &str = "/nonexistent/holder.sock";

    /// The process name `raw`.
    fn named(raw: &str) -> ProcessName {
        name(raw)
    }

    /// A harness over [`NAME`] spawning through `runner`, autostarting or not.
    fn over(runner: Box<dyn ProcessRunner>, autostart: bool) -> Harness {
        Harness::over(runner, &[
            spec(NAME, "sh", RestartPolicy::Never).with_autostart(autostart)
        ])
    }

    /// Feeds `bytes` to `pane`'s current process as output.
    fn feed(harness: &mut Harness, bytes: &[u8]) {
        let pane = PaneId::new(PANE);
        let generation = harness.supervisor.generations()[&pane];
        harness
            .supervisor
            .handle_output(pane, generation, ProcessOutput::Chunk(bytes.to_vec()));
    }

    /// The screen of the process `name` as a caller gets it: a holder read is
    /// performed here and settled back with the supervisor.
    fn output_of(harness: &mut Harness, name: &str) -> Option<String> {
        match harness.supervisor.screen(&named(name))? {
            ScreenReply::Ready(screen) => Some(screen),
            ScreenReply::ReadHolder(screen) => {
                let read = screen.read();
                Some(harness.supervisor.settle_holder_read(screen, read))
            },
        }
    }

    /// A handle to a process a holder at `socket` runs.
    struct HolderRunHandle {
        socket: PathBuf,
    }

    impl ProcessHandle for HolderRunHandle {
        fn holder_socket(&self) -> Option<&Path> {
            Some(&self.socket)
        }

        fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
            Ok(())
        }

        fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
            Ok(())
        }

        fn pause(&mut self) -> Result<(), PtyError> {
            Ok(())
        }

        fn resume(&mut self) -> Result<(), PtyError> {
            Ok(())
        }

        fn kill(&mut self) -> Result<(), PtyError> {
            Ok(())
        }
    }

    /// A runner whose every process runs under a holder at `socket`.
    struct HolderRunRunner {
        socket: PathBuf,
    }

    impl ProcessRunner for HolderRunRunner {
        fn spawn(
            &self,
            _request: SpawnRequest,
            _sink: Box<dyn OutputSink>,
        ) -> Result<Box<dyn ProcessHandle>, PtyError> {
            Ok(Box::new(HolderRunHandle {
                socket: self.socket.clone(),
            }))
        }
    }

    /// A started terminal run by a holder at `socket`, whose service-side
    /// screen shows [`SERVICE_SCREEN`].
    fn holder_run(socket: &Path) -> Harness {
        let mut harness = over(
            Box::new(HolderRunRunner {
                socket: socket.to_path_buf(),
            }),
            true,
        );
        harness.up();
        feed(&mut harness, SERVICE_SCREEN.as_bytes());
        harness
    }

    /// Serves one greeting of [`HOLDER_SCREEN`] on `listener`, as a holder
    /// answers a read-only client.
    fn serve_one_screen(listener: UnixListener) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            if ClientMessage::read_from(&mut stream).is_err() {
                return;
            }
            let _ = HolderMessage::Snapshot {
                repaint: HOLDER_SCREEN.as_bytes().to_vec(),
                size: PtySize::try_new(HOLDER_SIDE, HOLDER_SIDE).unwrap(),
                pid: FAKE_LAUNCHER_PID,
                attached: 0,
                run_start: OutputCursor::new(0),
            }
            .write_to(&mut stream);
        })
    }

    /// The warning notices raised so far.
    fn warnings(harness: &Harness) -> usize {
        harness
            .notices
            .lock()
            .unwrap()
            .recent(harness.supervisor.root())
            .iter()
            .filter(|notice| *notice.level() == NoticeLevel::Warning)
            .count()
    }

    /// A live process returns its screen, one not started yet returns empty,
    /// and only an unknown name returns nothing, all without a holder read.
    #[test]
    fn output_is_the_screen_empty_before_a_start_and_none_for_an_unknown_name() {
        let mut harness = over(Box::new(FakeRunner), false);
        assert_eq!(output_of(&mut harness, NAME), Some(String::new()));
        assert_eq!(output_of(&mut harness, UNKNOWN), None);

        harness
            .supervisor
            .start(&named(NAME), &LaunchEnvironment::default());
        feed(&mut harness, SERVICE_SCREEN.as_bytes());

        let Some(ScreenReply::Ready(screen)) = harness.supervisor.screen(&named(NAME)) else {
            panic!("a process without a holder is answered at once");
        };
        assert!(screen.contains(SERVICE_SCREEN));
    }

    /// The supervisor hands a holder-run process's screen to its caller to
    /// read rather than reading the socket itself, with the service's copy as
    /// the fallback.
    #[test]
    fn a_holder_run_screen_is_handed_to_the_caller_to_read() {
        let mut harness = holder_run(Path::new(UNSERVED_SOCKET));

        let screen = handed_out(&mut harness);

        assert_eq!(screen.pane(), PaneId::new(PANE));
        assert_eq!(screen.socket(), Path::new(UNSERVED_SOCKET));
        assert!(screen.fallback().contains(SERVICE_SCREEN));
    }

    /// The screen of a process a holder runs comes from that holder, whose
    /// emulator is authoritative, not from the service's mirror of it.
    #[test]
    fn output_of_a_holder_run_process_is_read_from_its_holder() {
        let scratch = tempfile::Builder::new()
            .prefix(SCRATCH_PREFIX)
            .tempdir_in(SHORT_TEMP_ROOT)
            .unwrap();
        let socket = scratch.path().join("holder.sock");
        let server = serve_one_screen(UnixListener::bind(&socket).unwrap());
        let mut harness = holder_run(&socket);

        let screen = output_of(&mut harness, NAME).unwrap();

        let _ = UnixStream::connect(&socket);
        server.join().unwrap();
        assert!(screen.contains(HOLDER_SCREEN), "read {screen:?}");
        assert_eq!(warnings(&harness), 0);
    }

    /// A holder that cannot be read is answered from the service's copy, with
    /// one warning per failure streak; a good read ends the streak.
    #[test]
    fn a_failed_holder_read_falls_back_and_warns_once_per_streak() {
        let mut harness = holder_run(Path::new(UNSERVED_SOCKET));

        assert!(
            output_of(&mut harness, NAME)
                .unwrap()
                .contains(SERVICE_SCREEN)
        );
        output_of(&mut harness, NAME);
        assert_eq!(warnings(&harness), 1, "a streak warns once");

        let screen = handed_out(&mut harness);
        harness
            .supervisor
            .settle_holder_read(screen, Ok(HOLDER_SCREEN.to_string()));
        output_of(&mut harness, NAME);
        assert_eq!(warnings(&harness), 2, "a new streak warns again");
    }

    /// The holder read the supervisor hands out for [`NAME`].
    fn handed_out(harness: &mut Harness) -> HolderScreen {
        match harness.supervisor.screen(&named(NAME)) {
            Some(ScreenReply::ReadHolder(screen)) => screen,
            other => panic!("answered at once: {other:?}"),
        }
    }

    /// A read settled after its process was respawned is answered but neither
    /// warns nor touches the failure streak, since it no longer describes the
    /// current process.
    #[test]
    fn a_stale_holder_read_is_dropped_without_a_warning() {
        let mut harness = holder_run(Path::new(UNSERVED_SOCKET));
        let respawned = handed_out(&mut harness);
        harness
            .supervisor
            .restart(&named(NAME), &LaunchEnvironment::default());
        let pane = PaneId::new(PANE);
        let generation = harness.supervisor.generations()[&pane];
        harness.supervisor.handle_output(
            pane,
            generation,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );
        assert_ne!(harness.supervisor.generations()[&pane], generation);
        let failure = || Err(PtyError::System(UNSERVED_SOCKET.to_string()));

        let answered = harness.supervisor.settle_holder_read(respawned, failure());

        assert!(
            answered.contains(SERVICE_SCREEN),
            "a stale read is still answered"
        );
        assert_eq!(warnings(&harness), 0);
        assert!(harness.supervisor.failing_screen_reads.is_empty());
    }

    #[test]
    fn input_reaches_a_live_process_and_is_refused_for_a_stopped_or_unknown_one() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let mut harness = over(
            Box::new(RecordingRunner {
                written: written.clone(),
            }),
            false,
        );
        let send = |harness: &mut Harness, name: &str| {
            harness.supervisor.send_input(&named(name), b"ls\r")
        };

        assert!(!send(&mut harness, NAME), "not running yet");
        assert!(!send(&mut harness, UNKNOWN), "no such process");
        harness
            .supervisor
            .start(&named(NAME), &LaunchEnvironment::default());
        assert!(send(&mut harness, NAME));
        assert_eq!(*written.lock().unwrap(), b"ls\r");
    }
}

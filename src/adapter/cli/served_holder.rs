use std::{os::unix::net::UnixStream, path::PathBuf, thread, time::Duration};

use crate::adapter::{
    holder::{
        ClientMessage, ClientRole, HolderControl, HolderMessage, client::greet, harness::Harness,
    },
    test_wait::{WAIT, wait_until},
};

/// How long the supervisor waits between asks to collect the exit.
const COLLECT_RETRY: Duration = Duration::from_millis(20);

/// A holder serving a real child on a thread, from the holder's own
/// [`Harness`], with a supervisor kept connected so the attach session under
/// test is never the holder's only client.
pub(super) struct Holder {
    /// The holder on its thread.
    harness: Harness,
    /// The test's supervisor connection.
    supervisor: UnixStream,
}

impl Holder {
    /// Starts a holder running `command` at the test terminal's size.
    pub(super) fn start(command: &str) -> Self {
        let harness = Harness::start(command);
        let socket = harness.config().socket().clone();
        let mut supervisor = UnixStream::connect(&socket).unwrap();
        greet(&mut supervisor, &socket, ClientRole::Supervisor, WAIT).unwrap();
        let mut drain = supervisor.try_clone().unwrap();
        drain.set_read_timeout(None).unwrap();
        thread::spawn(move || while HolderMessage::read_from(&mut drain).is_ok() {});
        Self {
            harness,
            supervisor,
        }
    }

    /// The holder's socket.
    pub(super) fn socket(&self) -> PathBuf {
        self.harness.config().socket().clone()
    }

    /// Writes `bytes` to the child as another, writable client would.
    pub(super) fn type_as_another_client(&self, bytes: &[u8]) {
        let socket = self.socket();
        let mut client = UnixStream::connect(&socket).unwrap();
        greet(
            &mut client,
            &socket,
            ClientRole::Attach { read_only: false },
            WAIT,
        )
        .unwrap();
        ClientMessage::Input(bytes.to_vec())
            .write_to(&mut client)
            .unwrap();
    }

    /// Kills the child when it still runs, collects its exit and joins the
    /// holder, checking it cleaned up.
    pub(super) fn finish(mut self) {
        ClientMessage::Control(HolderControl::Kill)
            .write_to(&mut self.supervisor)
            .unwrap();
        wait_until("the holder to finish", || {
            let _ = ClientMessage::CollectExit.write_to(&mut self.supervisor);
            thread::sleep(COLLECT_RETRY);
            self.harness.server_finished()
        });
        self.harness.finish_collected();
    }
}

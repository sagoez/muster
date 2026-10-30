use std::{
    net::Shutdown,
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError, mpsc},
    thread,
};

use getset::{Getters, MutGetters};
use tempfile::TempDir;

use super::attach_testing::{COLS, FAKE_PID, ROWS, scratch};
use crate::{
    adapter::{
        holder::{ClientMessage, HolderMessage},
        test_wait::{WAIT, wait_until},
    },
    domain::{
        pty::{PtySize, testing::size},
        value::{OutputCursor, Pid},
    },
};

/// A scripted holder: it answers the hello with a repaint, then records
/// what the client sends and writes what a test tells it to.
#[derive(Getters, MutGetters)]
pub(super) struct FakeHolder {
    /// Its socket.
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    socket: PathBuf,
    /// The client connection, once the client said hello.
    peer: Arc<Mutex<Option<UnixStream>>>,
    /// What the client sent, in order.
    received: mpsc::Receiver<ClientMessage>,
    /// The scratch directory, removed on drop.
    _scratch: TempDir,
}

impl FakeHolder {
    /// A holder greeting with `repaint` at the test terminal's size.
    pub(super) fn start(repaint: &[u8]) -> Self {
        Self::sized(repaint, size(ROWS, COLS))
    }

    /// A holder greeting with `repaint` rendered at `screen`.
    pub(super) fn sized(repaint: &[u8], screen: PtySize) -> Self {
        let scratch = scratch();
        let socket = scratch.path().join("f.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let peer: Arc<Mutex<Option<UnixStream>>> = Arc::new(Mutex::new(None));
        let (sender, received) = mpsc::channel();
        let shared = Arc::clone(&peer);
        let repaint = repaint.to_vec();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let hello = ClientMessage::read_from(&mut stream).unwrap();
            let _ = sender.send(hello);
            HolderMessage::Snapshot {
                repaint,
                size: screen,
                pid: Pid::try_new(FAKE_PID).unwrap(),
                attached: 0,
                run_start: OutputCursor::new(0),
                log_end: OutputCursor::new(0),
            }
            .write_to(&mut stream)
            .unwrap();
            *shared.lock().unwrap_or_else(PoisonError::into_inner) =
                Some(stream.try_clone().unwrap());
            while let Ok(message) = ClientMessage::read_from(&mut stream) {
                if sender.send(message).is_err() {
                    break;
                }
            }
        });
        Self {
            socket,
            peer,
            received,
            _scratch: scratch,
        }
    }

    /// The client connection, once it said hello.
    pub(super) fn peer(&self) -> UnixStream {
        let mut peer = None;
        wait_until("the client to say hello", || {
            peer = self
                .peer
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
                .map(|stream| stream.try_clone().unwrap());
            peer.is_some()
        });
        peer.unwrap()
    }

    /// Writes `message` to the client.
    pub(super) fn send(&self, message: HolderMessage) {
        message.write_to(&mut self.peer()).unwrap();
    }

    /// Breaks the connection.
    pub(super) fn close(&self) {
        self.peer().shutdown(Shutdown::Both).unwrap();
    }

    /// The next message the client sent.
    pub(super) fn next(&self) -> ClientMessage {
        self.received.recv_timeout(WAIT).unwrap()
    }

    /// Everything else the client sent, once it closed the connection.
    pub(super) fn rest(&self) -> Vec<ClientMessage> {
        self.received.iter().collect()
    }
}

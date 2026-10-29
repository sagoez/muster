use std::{
    fs::{self, DirBuilder, Permissions},
    os::unix::{
        fs::{DirBuilderExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crossbeam_channel::{Receiver, Sender, bounded};
use serde_json::Value;
use uuid::Uuid;

use super::{
    event::ServiceEvent, service_error::ServiceError, service_error_body::ServiceErrorBody,
    service_error_kind::ServiceErrorKind, service_request::ServiceRequest,
    service_response::ServiceResponse,
};
use crate::adapter::ipc;

/// Mode of the directory a socket is bound in before it is moved into place.
const STAGING_DIR_MODE: u32 = 0o700;
/// Mode of the service socket: only its owner may connect.
const SOCKET_MODE: u32 = 0o600;
/// Prefix of the private directory a socket is bound in.
const STAGING_PREFIX: &str = ".service-";
/// Longest the service waits, as it exits, for connections still writing
/// their answers.
const DRAIN_GRACE: Duration = Duration::from_secs(2);
/// Pause after an accept that failed, before trying again.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(50);
/// Answer to a request the service will not get to, as it is going away.
const SHUTTING_DOWN: &str = "the service is shutting down";
/// Prefix of the answer to a request that cannot be read.
const UNREADABLE_REQUEST: &str = "could not read the request";

/// Accepts client connections on the service socket and gives each a thread
/// that forwards its one request to the service loop and writes the answer
/// back, so no client I/O ever runs on the loop.
pub(super) struct Acceptor {
    /// The socket clients connect to.
    socket: PathBuf,
    /// Set once the service stops accepting.
    closing: Arc<AtomicBool>,
    /// The accepting thread.
    thread: Option<JoinHandle<()>>,
    /// Disconnects once the accepting thread and every connection it started
    /// have finished.
    connections: Receiver<()>,
}

impl Acceptor {
    /// Binds `socket` so only its owner can reach it, and starts accepting,
    /// forwarding each request to `events`.
    ///
    /// # Errors
    /// Returns a [`ServiceError`] when the socket cannot be bound.
    pub(super) fn bind(socket: &Path, events: Sender<ServiceEvent>) -> Result<Self, ServiceError> {
        let listener = bind_private(socket)?;
        let closing = Arc::new(AtomicBool::new(false));
        let (alive, connections) = bounded(0);
        let thread = spawn_acceptor(listener, events, Arc::clone(&closing), alive);
        Ok(Self {
            socket: socket.to_path_buf(),
            closing,
            thread: Some(thread),
            connections,
        })
    }

    /// Stops accepting and removes the socket, so no new client reaches this
    /// service. Connections already accepted carry on.
    pub(super) fn close(&mut self) {
        self.closing.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.socket);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        ipc::unlink(&self.socket);
    }

    /// Waits a bounded time for every accepted connection to finish writing
    /// its answer.
    pub(super) fn drain(self) {
        let _ = self.connections.recv_timeout(DRAIN_GRACE);
    }
}

/// The answer to a request the service will not get to.
fn busy() -> ServiceResponse {
    ServiceResponse::Error(
        ServiceErrorBody::builder()
            .kind(ServiceErrorKind::Busy)
            .message(SHUTTING_DOWN.to_string())
            .build(),
    )
}

/// Binds the service socket in a fresh private directory, makes it
/// owner-only, then moves it into place, so it is never reachable by another
/// user, even for a moment. A socket already at `socket` is stale: the service
/// lock proves no other service runs.
///
/// # Errors
/// Returns [`ServiceError::Bind`] when any step fails.
fn bind_private(socket: &Path) -> Result<UnixListener, ServiceError> {
    let bind_error = |source| ServiceError::Bind {
        path: socket.to_path_buf(),
        source,
    };
    let parent = socket.parent().unwrap_or(Path::new("."));
    let staging = parent.join(format!(
        "{STAGING_PREFIX}{}-{}",
        process::id(),
        Uuid::new_v4().simple()
    ));
    DirBuilder::new()
        .mode(STAGING_DIR_MODE)
        .create(&staging)
        .map_err(bind_error)?;
    let staged = staging.join(socket.file_name().unwrap_or(socket.as_os_str()));
    let moved = ipc::bind(&staged).and_then(|listener| {
        fs::set_permissions(&staged, Permissions::from_mode(SOCKET_MODE))?;
        fs::rename(&staged, socket)?;
        Ok(listener)
    });
    ipc::unlink(&staged);
    let _ = fs::remove_dir(&staging);
    moved.map_err(bind_error)
}

/// Accepts connections until `closing` is set, serving each on its own
/// thread. Every thread holds a clone of `alive`, so its receiver disconnects
/// once all of them have finished.
fn spawn_acceptor(
    listener: UnixListener,
    events: Sender<ServiceEvent>,
    closing: Arc<AtomicBool>,
    alive: Sender<()>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        for stream in listener.incoming() {
            if closing.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = stream else {
                // A failing accept, such as one out of descriptors, fails at
                // once again; waiting lets it clear rather than spin a core.
                thread::sleep(ACCEPT_BACKOFF);
                continue;
            };
            let events = events.clone();
            let alive = alive.clone();
            thread::spawn(move || {
                let _alive = alive;
                let _ = ipc::serve_connection(stream, |request: Value| answer(&events, request));
            });
        }
    })
}

/// Forwards one client request to the loop and waits for its answer. A
/// request that cannot be read is refused here, and one the loop drops
/// unanswered, as it exits, is told the service is going away. The wait is
/// unbounded on purpose: the loop holds the only reply sender, so it either
/// answers or drops it, and a client never sees an error for a request the
/// loop later carries out.
fn answer(events: &Sender<ServiceEvent>, request: Value) -> ServiceResponse {
    let request = match serde_json::from_value::<ServiceRequest>(request) {
        Ok(request) => request,
        Err(error) => {
            return ServiceResponse::Error(
                ServiceErrorBody::builder()
                    .kind(ServiceErrorKind::InvalidRequest)
                    .message(format!("{UNREADABLE_REQUEST}: {error}"))
                    .build(),
            );
        },
    };
    let (reply, answered) = bounded(1);
    if events
        .send(ServiceEvent::Request { request, reply })
        .is_err()
    {
        return busy();
    }
    answered.recv().unwrap_or_else(|_| busy())
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};

    use crossbeam_channel::unbounded;
    use tempfile::TempDir;

    use super::*;
    use crate::adapter::test_scratch::short_scratch;

    /// Prefix of every test scratch directory.
    const SCRATCH_PREFIX: &str = "ma-";
    /// Mode bits a permission check compares.
    const PERMISSION_BITS: u32 = 0o777;

    /// A scratch directory short enough to bind sockets in.
    fn scratch() -> TempDir {
        short_scratch(SCRATCH_PREFIX)
    }

    /// The socket is its owner's alone from the moment it is reachable, and a
    /// stale file left at its path by a crashed service is replaced.
    #[test]
    fn the_service_socket_is_private_and_replaces_a_stale_one() {
        let scratch = scratch();
        let socket = scratch.path().join("service.sock");
        drop(UnixListener::bind(&socket).unwrap());
        let (events, _received) = unbounded();

        let mut acceptor = Acceptor::bind(&socket, events).unwrap();

        let mode = fs::metadata(&socket).unwrap().permissions().mode() & PERMISSION_BITS;
        assert_eq!(mode, SOCKET_MODE);
        assert!(UnixStream::connect(&socket).is_ok());
        let leftovers: Vec<_> = fs::read_dir(scratch.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, vec!["service.sock"], "no staging dir is left");
        acceptor.close();
        assert!(!socket.exists());
    }

    /// A request the loop drops unanswered, as it exits, is told the service
    /// is going away rather than left hanging.
    #[test]
    fn a_request_the_loop_drops_is_answered_busy() {
        let (events, received) = unbounded();
        let dropping = thread::spawn(move || {
            drop(received.recv().unwrap());
        });

        let response = answer(&events, serde_json::json!("stop"));

        dropping.join().unwrap();
        let ServiceResponse::Error(refusal) = response else {
            panic!("a dropped request is refused, got {response:?}");
        };
        assert_eq!(*refusal.kind(), ServiceErrorKind::Busy);
    }

    /// A line that is not a request, as a client of another version may send,
    /// is answered with why rather than a closed connection.
    #[test]
    fn an_unreadable_request_is_answered_with_why() {
        let scratch = scratch();
        let socket = scratch.path().join("service.sock");
        let (events, _received) = unbounded();
        let mut acceptor = Acceptor::bind(&socket, events).unwrap();

        let mut stream = UnixStream::connect(&socket).unwrap();
        stream.write_all(b"{\"launch-rockets\":{}}\n").unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();

        let response: ServiceResponse = serde_json::from_str(&line).unwrap();
        let ServiceResponse::Error(refusal) = response else {
            panic!("an unreadable request is refused, got {response:?}");
        };
        assert_eq!(*refusal.kind(), ServiceErrorKind::InvalidRequest);
        acceptor.close();
        acceptor.drain();
    }
}

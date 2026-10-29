use std::{
    cmp::Ordering,
    env,
    fs::{File, OpenOptions, TryLockError},
    io,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

use super::{
    client_error::ClientError, probe::Probe, running_service::RunningService,
    service_log::ServiceLog, service_start::ServiceStart, service_status::ServiceStatus,
    unavailable_reason::UnavailableReason,
};
use crate::{
    adapter::{
        holder::{HolderDirs, HolderId, detached, reap},
        ipc::fits_a_socket,
        private_dir::{
            RUNTIME_DIR_ENV, STATE_HOME_ENV, check_socket_dir, prepare_socket_dir,
            prepare_state_dir, socket_dir, state_dir,
        },
        service::{
            Notice, ProjectStatus, ServiceErrorKind, ServiceRequest, ServiceResponse,
            ServiceVersion,
            protocol::{self, SERVICE_SUBCOMMAND},
        },
    },
    constants::APP_VERSION,
    domain::value::ProjectRoot,
};

/// Longest to wait for a service this client started to accept requests.
pub const SERVICE_READY_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest a service reply may take, and longest to wait for a service that
/// is exiting to be gone. Above the 10 s a `Stop` or `Replace` waits for
/// launches in flight, so a slow exit is not mistaken for a hung service.
const SERVICE_EXIT_TIMEOUT: Duration = Duration::from_secs(15);
/// Pause between attempts to reach a service that is starting or exiting.
const RETRY_INTERVAL: Duration = Duration::from_millis(20);
/// File in the state directory whose lock lets one client at a time start
/// the service.
const START_LOCK_FILE: &str = "service.start.lock";
/// Mode of the start lock: its owner's alone.
const START_LOCK_MODE: u32 = 0o600;
/// Longest a writer waits for a running service to hear that stored state
/// changed; a service that does not answer in time reads fresh state later.
const NOTIFY_TIMEOUT: Duration = Duration::from_secs(2);
/// The folder a path with no parent is relative to.
const CURRENT_DIR: &str = ".";

/// A client of this user's muster service, which it starts on demand and
/// replaces when the service runs an older muster version.
pub struct ServiceClient {
    /// The state directory the service owns.
    state_dir: PathBuf,
    /// Where a service this client starts binds its socket, which follows
    /// this client's runtime directory.
    own_socket: PathBuf,
    /// The muster binary a missing service is started from.
    binary: PathBuf,
}

impl ServiceClient {
    /// Connects to this user's service: starts it when its socket is absent
    /// or dead, and replaces it with this binary's when it runs an older
    /// version. Processes keep running across a replacement.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when the service cannot be started, does not
    /// answer, runs a newer version, or still runs another version after
    /// being replaced.
    pub fn connect() -> Result<Self, ClientError> {
        let client = Self::local()?;
        client.check_socket_room()?;
        let ours = ServiceVersion::current();
        let running = client.reach()?;
        match running.compared_with(&ours) {
            Ordering::Equal => return Ok(client),
            Ordering::Less => return Err(newer(running)),
            Ordering::Greater => client.replace(&running)?,
        }
        let replacement = client.reach()?;
        if replacement.compared_with(&ours) == Ordering::Equal {
            return Ok(client);
        }
        Err(ClientError::VersionMismatch {
            ours: APP_VERSION.to_string(),
            theirs: replacement
                .version()
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
        })
    }

    /// Whether the service runs, its version and the projects it has loaded,
    /// without starting or replacing it.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when a service holds the socket but does not
    /// answer.
    pub fn status() -> Result<ServiceStatus, ClientError> {
        let client = Self::local()?;
        let running = match client.probe()? {
            Probe::Running(running) => Some(running),
            Probe::Absent | Probe::Exiting => None,
        };
        let projects = match running {
            Some(_) => match client.exchange(&ServiceRequest::Ps { project: None })? {
                ServiceResponse::Status(statuses) => statuses
                    .iter()
                    .map(|status| status.project().root().as_ref().clone())
                    .collect(),
                other @ (ServiceResponse::Hello { .. }
                | ServiceResponse::Ok
                | ServiceResponse::Batch(_)
                | ServiceResponse::Log(_)
                | ServiceResponse::Located(_)
                | ServiceResponse::Screen(_)
                | ServiceResponse::Notices(_)
                | ServiceResponse::Waited(_)
                | ServiceResponse::Events(_)
                | ServiceResponse::Error(_)) => {
                    return Err(ClientError::UnexpectedAnswer((&other).into()));
                },
            },
            None => Vec::new(),
        };
        Ok(ServiceStatus::builder()
            .running(running.is_some())
            .version(
                running
                    .as_ref()
                    .and_then(|running| running.version().clone()),
            )
            .binary(
                running
                    .as_ref()
                    .and_then(|running| running.binary().clone()),
            )
            .client_version(ServiceVersion::current())
            .socket(client.shown_socket())
            .log(client.log().path().clone())
            .projects(projects)
            .build())
    }

    /// The status of `project` when a running service has it loaded, without
    /// starting the service or loading the project.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when a service holds the socket but does not
    /// answer.
    pub fn loaded_status(project: &ProjectRoot) -> Result<Option<ProjectStatus>, ClientError> {
        let client = Self::local()?;
        if !matches!(client.probe()?, Probe::Running(_)) {
            return Ok(None);
        }
        match client.exchange(&ServiceRequest::Ps { project: None })? {
            ServiceResponse::Status(statuses) => Ok(statuses
                .into_iter()
                .find(|status| status.project().root() == project)),
            other @ (ServiceResponse::Hello { .. }
            | ServiceResponse::Ok
            | ServiceResponse::Batch(_)
            | ServiceResponse::Log(_)
            | ServiceResponse::Located(_)
            | ServiceResponse::Screen(_)
            | ServiceResponse::Notices(_)
            | ServiceResponse::Waited(_)
            | ServiceResponse::Events(_)
            | ServiceResponse::Error(_)) => Err(ClientError::UnexpectedAnswer((&other).into())),
        }
    }

    /// The recent notices a running service raised about `project` and
    /// itself, without starting the service; none when no service runs.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when a service holds the socket but does not
    /// answer.
    pub fn running_notices(project: &ProjectRoot) -> Result<Option<Vec<Notice>>, ClientError> {
        let client = Self::local()?;
        if !matches!(client.probe()?, Probe::Running(_)) {
            return Ok(None);
        }
        match client.exchange(&ServiceRequest::Notices {
            project: project.clone(),
        })? {
            ServiceResponse::Notices(notices) => Ok(Some(notices)),
            other @ (ServiceResponse::Hello { .. }
            | ServiceResponse::Ok
            | ServiceResponse::Batch(_)
            | ServiceResponse::Log(_)
            | ServiceResponse::Status(_)
            | ServiceResponse::Located(_)
            | ServiceResponse::Screen(_)
            | ServiceResponse::Waited(_)
            | ServiceResponse::Events(_)
            | ServiceResponse::Error(_)) => Err(ClientError::UnexpectedAnswer((&other).into())),
        }
    }

    /// Sends `request` to a service that is running, best effort: absence,
    /// a refusal, another version or a slow answer are all ignored, and no
    /// service is started.
    pub fn notify_if_running(request: &ServiceRequest) {
        if let Ok(client) = Self::local() {
            let _ = client.send_once(request, NOTIFY_TIMEOUT);
        }
    }

    /// The client of the service under this user's state directory, started
    /// from this binary.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when there is no state directory or the binary
    /// cannot be found.
    fn local() -> Result<Self, ClientError> {
        let state_dir = state_dir()?;
        let binary = env::current_exe().map_err(|error| ClientError::ServiceUnavailable {
            reason: UnavailableReason::NoBinary(error),
            log: protocol::log_path(&state_dir),
            tail: Vec::new(),
        })?;
        Ok(Self::under(state_dir, &socket_dir(), binary))
    }

    /// The client of the service of `state_dir`, which a service it starts
    /// from `binary` binds in `socket_dir`.
    fn under(state_dir: PathBuf, socket_dir: &Path, binary: PathBuf) -> Self {
        Self {
            own_socket: protocol::socket_path(socket_dir, &state_dir),
            state_dir,
            binary,
        }
    }

    /// The socket to reach the service at: the one the running service
    /// published in the state directory, else where a service this client
    /// starts binds. Its directory is checked private to this user first,
    /// created when it is this client's own, so no request ever goes to a
    /// socket another user could have planted. A published socket whose
    /// directory is gone is stale and passed over.
    ///
    /// # Errors
    /// Returns an error of kind [`io::ErrorKind::PermissionDenied`] naming
    /// the directory when it is not private to this user, or the I/O error
    /// from creating or inspecting it.
    fn socket(&self) -> io::Result<PathBuf> {
        if let Some(published) = protocol::published_socket(&self.state_dir) {
            match check_socket_dir(parent_dir(&published)) {
                Ok(()) => return Ok(published),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {},
                Err(error) => return Err(error),
            }
        }
        prepare_socket_dir(parent_dir(&self.own_socket))?;
        Ok(self.own_socket.clone())
    }

    /// The socket shown to a person: the one requests go to, or this
    /// client's own when that cannot be told.
    fn shown_socket(&self) -> PathBuf {
        self.socket().unwrap_or_else(|_| self.own_socket.clone())
    }

    /// Sends `request` once to the socket [`Self::socket`] trusts, waiting at
    /// most `timeout` for the answer.
    ///
    /// # Errors
    /// Returns the refusal of the socket's directory or peer, or the failure
    /// of the exchange.
    fn send_once(
        &self,
        request: &ServiceRequest,
        timeout: Duration,
    ) -> io::Result<ServiceResponse> {
        protocol::send_within(&self.socket()?, request, timeout)
    }

    /// Refuses a runtime directory too deep for the service's socket, or a
    /// state directory too deep for a holder's, to fit in a Unix socket
    /// address, before a service is started that could never listen. Every
    /// path the service binds is checked, the name it binds first included.
    ///
    /// # Errors
    /// Returns [`ClientError::SocketPathTooLong`] naming the first that does
    /// not fit and the variable that moves it.
    fn check_socket_room(&self) -> Result<(), ClientError> {
        let service = [
            self.own_socket.clone(),
            protocol::staging_socket(&self.own_socket),
        ]
        .into_iter()
        .map(|socket| (socket, RUNTIME_DIR_ENV));
        let holder = HolderId::generate().ok().map(|id| {
            (
                HolderDirs::under(&self.state_dir).socket_path(&id),
                STATE_HOME_ENV,
            )
        });
        match service
            .chain(holder)
            .find(|(socket, _)| !fits_a_socket(socket))
        {
            Some((socket, variable)) => Err(ClientError::SocketPathTooLong { socket, variable }),
            None => Ok(()),
        }
    }

    /// The service log.
    fn log(&self) -> ServiceLog {
        ServiceLog::new(protocol::log_path(&self.state_dir))
    }

    /// Sends `request` and returns the answer. A request the service could
    /// not take, because it was not running or was exiting, is sent once more
    /// to a service started afresh.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when the service cannot be reached or answers
    /// late.
    pub fn send(&self, request: &ServiceRequest) -> Result<ServiceResponse, ClientError> {
        self.send_within(request, SERVICE_EXIT_TIMEOUT)
    }

    /// Sends `request`, which the service answers only after waiting up to
    /// `wait`, as [`Self::send`] does, allowing for that wait.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when the service cannot be reached or answers
    /// late.
    pub fn send_waiting(
        &self,
        request: &ServiceRequest,
        wait: Duration,
    ) -> Result<ServiceResponse, ClientError> {
        self.send_within(request, wait.saturating_add(SERVICE_EXIT_TIMEOUT))
    }

    /// Sends `request`, waiting at most `timeout` for each answer, and sends
    /// it once more to a service started afresh when it could not be taken.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when the service cannot be reached or answers
    /// late.
    fn send_within(
        &self,
        request: &ServiceRequest,
        timeout: Duration,
    ) -> Result<ServiceResponse, ClientError> {
        match self.send_once(request, timeout) {
            Ok(ServiceResponse::Error(body)) if *body.kind() == ServiceErrorKind::Busy => {
                self.reach()?;
                self.try_send(request, timeout)
            },
            Err(error) if not_listening(&error) => {
                self.reach()?;
                self.try_send(request, timeout)
            },
            Ok(response) => Ok(response),
            Err(source) => Err(self.io_error(source)),
        }
    }

    /// Asks the service to exit, leaving every process running. Returns
    /// whether a service was running. The request goes straight to the
    /// socket, with no hello first, so a service of any version is stopped.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when the service does not exit in time.
    pub fn stop() -> Result<bool, ClientError> {
        let client = Self::local()?;
        match client.send_once(&ServiceRequest::Stop, SERVICE_EXIT_TIMEOUT) {
            Ok(ServiceResponse::Ok) => Ok(true),
            Ok(ServiceResponse::Error(body)) if *body.kind() == ServiceErrorKind::Busy => Ok(true),
            Ok(other) => Err(ClientError::UnexpectedAnswer((&other).into())),
            Err(error) if not_listening(&error) => Ok(false),
            Err(error) if gone(&error) => Ok(true),
            Err(source) => Err(client.io_error(source)),
        }
    }

    /// Sends `request` once, waiting at most `timeout` for the answer, with
    /// no service started or waited out.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when the exchange fails.
    fn try_send(
        &self,
        request: &ServiceRequest,
        timeout: Duration,
    ) -> Result<ServiceResponse, ClientError> {
        self.send_once(request, timeout)
            .map_err(|source| self.io_error(source))
    }

    /// Sends `request` once.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when the exchange fails.
    fn exchange(&self, request: &ServiceRequest) -> Result<ServiceResponse, ClientError> {
        self.try_send(request, SERVICE_EXIT_TIMEOUT)
    }

    /// The service answering at the socket, starting one when none is, and
    /// waiting out one that is exiting.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when no service answers in time.
    fn reach(&self) -> Result<RunningService, ClientError> {
        let deadline = Instant::now() + SERVICE_EXIT_TIMEOUT;
        loop {
            match self.probe()? {
                Probe::Running(running) => return Ok(running),
                Probe::Absent => return self.start(),
                Probe::Exiting if Instant::now() >= deadline => {
                    return Err(self
                        .unavailable(UnavailableReason::StillExiting(SERVICE_EXIT_TIMEOUT), None));
                },
                Probe::Exiting => thread::sleep(RETRY_INTERVAL),
            }
        }
    }

    /// What answers at the socket.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when something accepts but does not answer.
    fn probe(&self) -> Result<Probe, ClientError> {
        let hello = ServiceRequest::Hello {
            version: ServiceVersion::current(),
        };
        match self.send_once(&hello, SERVICE_EXIT_TIMEOUT) {
            Ok(ServiceResponse::Hello { version, binary }) => Ok(Probe::Running(
                RunningService::builder()
                    .version(Some(version))
                    .binary(Some(binary))
                    .build(),
            )),
            Ok(ServiceResponse::Error(body)) if *body.kind() == ServiceErrorKind::Busy => {
                Ok(Probe::Exiting)
            },
            Ok(ServiceResponse::Error(body))
                if *body.kind() == ServiceErrorKind::InvalidRequest =>
            {
                Ok(Probe::Running(
                    RunningService::builder().version(None).binary(None).build(),
                ))
            },
            Ok(other) => Err(ClientError::UnexpectedAnswer((&other).into())),
            Err(error) if not_listening(&error) => Ok(Probe::Absent),
            Err(error) if gone(&error) => Ok(Probe::Exiting),
            Err(source) => Err(self.io_error(source)),
        }
    }

    /// Starts the service from this binary and waits until it answers,
    /// letting one client at a time start it so racing commands start one
    /// service between them. A spawned service that exits at once because
    /// another service holds the state directory is not spawned again; the
    /// client waits for that other service, or for one started after it has
    /// gone, and only once that wait ends with no answer starts one more.
    ///
    /// # Errors
    /// Returns a [`ClientError`], with what the service logged since it was
    /// started, when it exits with a failure or does not answer in time.
    fn start(&self) -> Result<RunningService, ClientError> {
        prepare_state_dir(&self.state_dir).map_err(|source| {
            self.unavailable(
                UnavailableReason::StateDir {
                    path: self.state_dir.clone(),
                    source,
                },
                None,
            )
        })?;
        if let ServiceStart::Running(running) = self.start_once()? {
            return Ok(running);
        }
        let deadline = Instant::now() + SERVICE_EXIT_TIMEOUT;
        while Instant::now() < deadline {
            if let Probe::Running(running) = self.probe()? {
                return Ok(running);
            }
            thread::sleep(RETRY_INTERVAL);
        }
        match self.start_once()? {
            ServiceStart::Running(running) => Ok(running),
            ServiceStart::Lost => {
                Err(self.unavailable(UnavailableReason::HeldElsewhere(SERVICE_EXIT_TIMEOUT), None))
            },
        }
    }

    /// Under the start lock, spawns one service unless one already answers,
    /// and waits until it answers or exits.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when the service exits with a failure, does not
    /// answer by [`SERVICE_READY_TIMEOUT`], or cannot be spawned.
    fn start_once(&self) -> Result<ServiceStart, ClientError> {
        let _lock = self.start_lock()?;
        if let Probe::Running(running) = self.probe()? {
            return Ok(ServiceStart::Running(running));
        }
        let attempt = Some(self.log().end());
        let mut service = self.spawn_service()?;
        let deadline = Instant::now() + SERVICE_READY_TIMEOUT;
        loop {
            match self.probe() {
                Ok(Probe::Running(running)) => {
                    reap(service);
                    return Ok(ServiceStart::Running(running));
                },
                Ok(Probe::Absent | Probe::Exiting) => {},
                Err(error) => {
                    reap(service);
                    return Err(error);
                },
            }
            match service.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(ServiceStart::Lost),
                Ok(Some(status)) => {
                    return Err(self.unavailable(UnavailableReason::Exited(status), attempt));
                },
                Ok(None) | Err(_) => {},
            }
            if Instant::now() >= deadline {
                reap(service);
                return Err(
                    self.unavailable(UnavailableReason::NoAnswer(SERVICE_READY_TIMEOUT), attempt)
                );
            }
            thread::sleep(RETRY_INTERVAL);
        }
    }

    /// Starts `<binary> __service` detached from this terminal and session.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when it cannot be started.
    fn spawn_service(&self) -> Result<Child, ClientError> {
        let mut command = Command::new(&self.binary);
        command.arg(SERVICE_SUBCOMMAND);
        detached(&mut command).spawn().map_err(|source| {
            self.unavailable(
                UnavailableReason::Spawn {
                    binary: self.binary.clone(),
                    subcommand: SERVICE_SUBCOMMAND,
                    source,
                },
                None,
            )
        })
    }

    /// Takes the start lock, waiting for another client's start. Dropping
    /// the file releases it.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when the lock cannot be opened or is still held
    /// after [`SERVICE_EXIT_TIMEOUT`].
    fn start_lock(&self) -> Result<File, ClientError> {
        let path = self.state_dir.join(START_LOCK_FILE);
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(START_LOCK_MODE)
            .open(&path)
            .map_err(|source| {
                self.unavailable(
                    UnavailableReason::StartLock {
                        path: path.clone(),
                        source,
                    },
                    None,
                )
            })?;
        let deadline = Instant::now() + SERVICE_EXIT_TIMEOUT;
        loop {
            match lock.try_lock() {
                Ok(()) => return Ok(lock),
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    thread::sleep(RETRY_INTERVAL);
                },
                Err(TryLockError::WouldBlock) => {
                    return Err(self.unavailable(
                        UnavailableReason::StartLockBusy(SERVICE_EXIT_TIMEOUT),
                        None,
                    ));
                },
                Err(TryLockError::Error(source)) => {
                    return Err(
                        self.unavailable(UnavailableReason::StartLock { path, source }, None)
                    );
                },
            }
        }
    }

    /// Asks the older service `running` to exit so this binary's can take
    /// over, and waits until it has.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when it does not exit in time.
    fn replace(&self, running: &RunningService) -> Result<(), ClientError> {
        match self.send_once(&ServiceRequest::Replace, SERVICE_EXIT_TIMEOUT) {
            Ok(ServiceResponse::Ok) => Ok(()),
            Ok(ServiceResponse::Error(body)) if *body.kind() == ServiceErrorKind::Busy => Ok(()),
            Ok(other) => Err(ClientError::UnexpectedAnswer((&other).into())),
            Err(error) if not_listening(&error) || gone(&error) => Ok(()),
            Err(source) => Err(self.unavailable(
                UnavailableReason::DidNotMakeWay {
                    theirs: running
                        .version()
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                    ours: APP_VERSION,
                    source,
                },
                None,
            )),
        }
    }

    /// The failure of a service that is not available for `reason`, with
    /// what the service logged since `attempt`, the log's end when this
    /// client started it.
    fn unavailable(&self, reason: UnavailableReason, attempt: Option<u64>) -> ClientError {
        let log = self.log();
        let tail = attempt.map(|from| log.tail_since(from)).unwrap_or_default();
        ClientError::ServiceUnavailable {
            reason,
            log: log.path().clone(),
            tail,
        }
    }

    /// The failure of an exchange that failed with `source`.
    fn io_error(&self, source: io::Error) -> ClientError {
        ClientError::ServiceIo {
            socket: self.shown_socket(),
            source,
        }
    }
}

/// The refusal to use the newer service `running`.
fn newer(running: RunningService) -> ClientError {
    ClientError::ServiceNewer {
        ours: APP_VERSION.to_string(),
        theirs: running
            .version()
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        binary: running.binary().clone(),
    }
}

/// Whether `error` means nothing listens at the socket.
fn not_listening(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

/// Whether `error` means the service closed the connection without
/// answering, as one exiting does.
fn gone(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset | io::ErrorKind::BrokenPipe
    )
}

/// The directory holding `socket`.
fn parent_dir(socket: &Path) -> &Path {
    socket.parent().unwrap_or(Path::new(CURRENT_DIR))
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, Permissions},
        os::unix::{fs::PermissionsExt, net::UnixListener},
    };

    use super::*;
    use crate::adapter::{private_dir::GROUP_OTHER_BITS, test_scratch::short_scratch};

    /// Prefix of each test's scratch directory.
    const SCRATCH_PREFIX: &str = "msc-";
    /// A directory mode every user may enter and write.
    const OPEN_DIR_MODE: u32 = 0o777;

    /// A client over a fresh state directory in `scratch`, whose own socket
    /// goes in `run`.
    fn client_in(scratch: &Path) -> ServiceClient {
        let state_dir = scratch.join("state");
        fs::create_dir(&state_dir).unwrap();
        ServiceClient::under(state_dir, &scratch.join("run"), PathBuf::from(CURRENT_DIR))
    }

    /// A listener at `socket` that never blocks to accept.
    fn listener_at(socket: &Path) -> UnixListener {
        let listener = UnixListener::bind(socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        listener
    }

    /// Whether anything connected to `listener`.
    fn reached(listener: &UnixListener) -> bool {
        match listener.accept() {
            Ok(_) => true,
            Err(error) => {
                assert_eq!(error.kind(), io::ErrorKind::WouldBlock, "{error}");
                false
            },
        }
    }

    /// A socket directory every user may write is refused, naming it, before
    /// a byte reaches the listener planted there, by a probe, a request and a
    /// best-effort notice alike.
    #[test]
    fn a_socket_dir_others_can_write_is_refused_before_connecting() {
        let scratch = short_scratch(SCRATCH_PREFIX);
        let client = client_in(scratch.path());
        let dir = scratch.path().join("run");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, Permissions::from_mode(OPEN_DIR_MODE)).unwrap();
        let planted = listener_at(&client.own_socket);

        let probed = client.probe().map(|_| ()).unwrap_err();
        let sent = client
            .send(&ServiceRequest::Ps { project: None })
            .unwrap_err();
        let _ = client.send_once(&ServiceRequest::SettingsChanged, NOTIFY_TIMEOUT);

        for error in [probed.to_string(), sent.to_string()] {
            assert!(error.contains(&*dir.to_string_lossy()), "{error}");
            assert!(error.contains("not a directory private"), "{error}");
        }
        assert!(!reached(&planted), "a request reached the planted socket");
    }

    /// A published socket in a directory others can write is refused too,
    /// rather than passed over for the client's own.
    #[test]
    fn a_published_socket_in_an_open_dir_is_refused() {
        let scratch = short_scratch(SCRATCH_PREFIX);
        let client = client_in(scratch.path());
        let open = scratch.path().join("open");
        fs::create_dir(&open).unwrap();
        fs::set_permissions(&open, Permissions::from_mode(OPEN_DIR_MODE)).unwrap();
        let socket = open.join("s.sock");
        let planted = listener_at(&socket);
        protocol::publish_socket(&client.state_dir, &socket).unwrap();

        let error = client.probe().map(|_| ()).unwrap_err().to_string();

        assert!(error.contains(&*open.to_string_lossy()), "{error}");
        assert!(!reached(&planted), "a request reached the planted socket");
    }

    /// The published socket is used over the client's own, and a stale one
    /// whose directory is gone gives way to the client's own, created
    /// private.
    #[test]
    fn the_published_socket_is_preferred_until_its_dir_is_gone() {
        let scratch = short_scratch(SCRATCH_PREFIX);
        let client = client_in(scratch.path());
        let elsewhere = scratch.path().join("elsewhere");
        let published = elsewhere.join("s.sock");
        protocol::publish_socket(&client.state_dir, &published).unwrap();

        assert_eq!(client.socket().unwrap(), client.own_socket);
        prepare_socket_dir(&elsewhere).unwrap();
        assert_eq!(client.socket().unwrap(), published);
        let own_dir = scratch.path().join("run");
        let mode = fs::metadata(&own_dir).unwrap().permissions().mode();
        assert_eq!(mode & GROUP_OTHER_BITS, 0);
    }
}

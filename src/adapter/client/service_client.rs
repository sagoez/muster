use std::{
    cmp::Ordering,
    env,
    fs::{File, OpenOptions, TryLockError},
    io,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

use semver::Version;

use super::{
    client_error::ClientError, probe::Probe, running_service::RunningService,
    service_log::ServiceLog, service_start::ServiceStart, service_status::ServiceStatus,
    unavailable_reason::UnavailableReason,
};
use crate::{
    adapter::{
        holder::{detached, reap},
        private_dir::{prepare_state_dir, state_dir},
        service::{
            ProjectStatus, ServiceErrorKind, ServiceRequest, ServiceResponse,
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

/// A client of this user's muster service, which it starts on demand and
/// replaces when the service runs an older muster version.
pub struct ServiceClient {
    /// The state directory the service owns.
    state_dir: PathBuf,
    /// The service socket.
    socket: PathBuf,
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
        let ours = ours();
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
            theirs: replacement.version().clone().unwrap_or_default(),
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
                other => return Err(ClientError::UnexpectedAnswer((&other).into())),
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
            .client_version(APP_VERSION.to_string())
            .socket(client.socket.clone())
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
            other => Err(ClientError::UnexpectedAnswer((&other).into())),
        }
    }

    /// Sends `request` to a service that is running, best effort: absence,
    /// a refusal, another version or a slow answer are all ignored, and no
    /// service is started.
    pub fn notify_if_running(request: &ServiceRequest) {
        if let Ok(client) = Self::local() {
            let _ = protocol::send_within(&client.socket, request, NOTIFY_TIMEOUT);
        }
    }

    /// The client of the service under this user's state directory, started
    /// from this binary.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when there is no state directory or the binary
    /// cannot be found.
    fn local() -> Result<Self, ClientError> {
        let state_dir = state_dir().ok_or(ClientError::NoStateDir)?;
        let socket = protocol::socket_path(&state_dir);
        let binary = env::current_exe().map_err(|error| ClientError::ServiceUnavailable {
            reason: UnavailableReason::NoBinary(error),
            log: protocol::log_path(&state_dir),
            tail: Vec::new(),
        })?;
        Ok(Self {
            state_dir,
            socket,
            binary,
        })
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
        match protocol::send_within(&self.socket, request, timeout) {
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
        match protocol::send_within(&client.socket, &ServiceRequest::Stop, SERVICE_EXIT_TIMEOUT) {
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
        protocol::send_within(&self.socket, request, timeout)
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
            version: APP_VERSION.to_string(),
        };
        match protocol::send_within(&self.socket, &hello, SERVICE_EXIT_TIMEOUT) {
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
        match protocol::send_within(&self.socket, &ServiceRequest::Replace, SERVICE_EXIT_TIMEOUT) {
            Ok(ServiceResponse::Ok) => Ok(()),
            Ok(ServiceResponse::Error(body)) if *body.kind() == ServiceErrorKind::Busy => Ok(()),
            Ok(other) => Err(ClientError::UnexpectedAnswer((&other).into())),
            Err(error) if not_listening(&error) || gone(&error) => Ok(()),
            Err(source) => Err(self.unavailable(
                UnavailableReason::DidNotMakeWay {
                    theirs: running.version().clone().unwrap_or_default(),
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
            socket: self.socket.clone(),
            source,
        }
    }
}

/// This binary's version.
fn ours() -> Version {
    Version::parse(APP_VERSION).unwrap_or_else(|_| Version::new(0, 0, 0))
}

/// The refusal to use the newer service `running`.
fn newer(running: RunningService) -> ClientError {
    ClientError::ServiceNewer {
        ours: APP_VERSION.to_string(),
        theirs: running.version().clone().unwrap_or_default(),
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

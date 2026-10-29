use std::{
    fs,
    path::PathBuf,
    thread::{self, JoinHandle},
    time::Duration,
};

use tempfile::TempDir;

use super::service_client::ServiceClient;
use crate::adapter::{
    ipc,
    private_dir::prepare_socket_dir,
    service::{ServiceRequest, ServiceResponse, ServiceVersion, protocol},
    test_scratch::short_scratch,
};

/// Prefix of each served service's scratch directory.
const SCRATCH_PREFIX: &str = "msv-";
/// Longest the server may take to answer the request that stops it.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
/// The state directory under the scratch directory.
const STATE_DIR: &str = "state";
/// The socket directory under the scratch directory.
const SOCKET_DIR: &str = "run";

/// Makes a client of a [`ServedService`].
pub(crate) type Connector = Box<dyn Fn() -> ServiceClient + Send + Sync>;

/// A service of this version on a scratch socket, answering a hello as one
/// and every other request with the answer a test scripted, and recording
/// what it was asked until it is stopped.
pub(crate) struct ServedService {
    /// The scratch directory holding its state and socket directories.
    scratch: TempDir,
    /// Its server thread, which hands back what it was asked once stopped.
    server: Option<JoinHandle<Vec<ServiceRequest>>>,
}

impl ServedService {
    /// A service answering every request but a hello with `answer`.
    pub(crate) fn answering(answer: ServiceResponse) -> Self {
        let scratch = short_scratch(SCRATCH_PREFIX);
        fs::create_dir(scratch.path().join(STATE_DIR)).unwrap();
        prepare_socket_dir(&scratch.path().join(SOCKET_DIR)).unwrap();
        let listener = ipc::bind(&socket_in(&scratch)).unwrap();
        let server = thread::spawn(move || {
            let mut asked = Vec::new();
            for stream in listener.incoming() {
                let mut stopped = false;
                ipc::serve_connection(stream.unwrap(), |request: ServiceRequest| match request {
                    ServiceRequest::Stop => {
                        stopped = true;
                        ServiceResponse::Ok
                    },
                    ServiceRequest::Hello { .. } => {
                        asked.push(request);
                        ServiceResponse::Hello {
                            version: ServiceVersion::current(),
                            binary: PathBuf::new(),
                        }
                    },
                    _ => {
                        asked.push(request);
                        answer.clone()
                    },
                })
                .unwrap();
                if stopped {
                    break;
                }
            }
            asked
        });
        Self {
            scratch,
            server: Some(server),
        }
    }

    /// What makes a client of this service, which can never start another.
    pub(crate) fn connector(&self) -> Connector {
        let (state_dir, socket_dir) = (self.state_dir(), self.socket_dir());
        Box::new(move || ServiceClient::scratch(state_dir.clone(), &socket_dir))
    }

    /// Stops the service and hands back every request it was asked, in
    /// order.
    pub(crate) fn asked(mut self) -> Vec<ServiceRequest> {
        self.stop()
    }

    /// The state directory the service owns.
    fn state_dir(&self) -> PathBuf {
        self.scratch.path().join(STATE_DIR)
    }

    /// The directory its socket is in.
    fn socket_dir(&self) -> PathBuf {
        self.scratch.path().join(SOCKET_DIR)
    }

    /// Its socket.
    fn socket(&self) -> PathBuf {
        socket_in(&self.scratch)
    }

    /// Stops the server thread and hands back what it was asked.
    fn stop(&mut self) -> Vec<ServiceRequest> {
        let server = self.server.take().unwrap();
        protocol::send_within(&self.socket(), &ServiceRequest::Stop, STOP_TIMEOUT).unwrap();
        server.join().unwrap()
    }
}

/// The socket a service in `scratch` listens on.
fn socket_in(scratch: &TempDir) -> PathBuf {
    protocol::socket_path(
        &scratch.path().join(SOCKET_DIR),
        &scratch.path().join(STATE_DIR),
    )
}

impl Drop for ServedService {
    /// Stops a server still running, quietly, so a failing test still ends
    /// its thread.
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            let _ = protocol::send_within(&self.socket(), &ServiceRequest::Stop, STOP_TIMEOUT);
            let _ = server.join();
        }
    }
}

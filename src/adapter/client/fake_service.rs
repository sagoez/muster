use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use super::{
    client_error::ClientError, service_access::ServiceAccess, service_client::ServiceClient,
    service_status::ServiceStatus,
};
use crate::{
    adapter::{
        no_state_dir::NoStateDir,
        service::{Notice, ProjectStatus, ServiceRequest},
    },
    domain::value::ProjectRoot,
};

/// A service a test never reaches: connecting fails as an unreachable service
/// would and is counted, nothing is loaded, notices are answered as `running`
/// says, and every request it is told is recorded.
#[derive(Default)]
pub(crate) struct FakeService {
    /// The live notices of a running service, or none when none runs.
    running: Option<Vec<Notice>>,
    /// Every request the service was told, in order.
    told: Mutex<Vec<ServiceRequest>>,
    /// How many times a connection was attempted.
    connects: AtomicUsize,
}

impl FakeService {
    /// A service that runs and holds `notices`.
    pub(crate) fn running(notices: Vec<Notice>) -> Self {
        Self {
            running: Some(notices),
            ..Self::default()
        }
    }

    /// Every request the service was told, in order.
    pub(crate) fn told(&self) -> Vec<ServiceRequest> {
        self.told.lock().unwrap().clone()
    }

    /// How many times a connection was attempted.
    pub(crate) fn connects(&self) -> usize {
        self.connects.load(Ordering::SeqCst)
    }
}

impl ServiceAccess for FakeService {
    fn connect(&self) -> Result<ServiceClient, ClientError> {
        self.connects.fetch_add(1, Ordering::SeqCst);
        Err(ClientError::NoStateDir(NoStateDir))
    }

    fn loaded_status(&self, _project: &ProjectRoot) -> Result<Option<ProjectStatus>, ClientError> {
        Ok(None)
    }

    fn running_notices(&self, _project: &ProjectRoot) -> Result<Option<Vec<Notice>>, ClientError> {
        Ok(self.running.clone())
    }

    fn status(&self) -> Result<ServiceStatus, ClientError> {
        Err(ClientError::NoStateDir(NoStateDir))
    }

    fn stop(&self) -> Result<bool, ClientError> {
        Ok(false)
    }

    fn notify(&self, request: &ServiceRequest) {
        self.told.lock().unwrap().push(request.clone());
    }
}

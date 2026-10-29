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

/// A service a test must never start: connecting fails the test, and it
/// answers notices only as `running` says.
pub struct UnstartedService {
    /// The live notices of a running service, or none when none runs.
    pub(super) running: Option<Vec<Notice>>,
}

impl ServiceAccess for UnstartedService {
    fn connect(&self) -> Result<ServiceClient, ClientError> {
        panic!("the service was started");
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

    fn notify(&self, _request: &ServiceRequest) {}
}

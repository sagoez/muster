use crate::{
    adapter::{
        client::{ClientError, ServiceAccess, ServiceClient, ServiceStatus},
        no_state_dir::NoStateDir,
        service::{Notice, ProjectStatus, ServiceRequest},
    },
    domain::value::ProjectRoot,
};

/// A service that is never running: connecting fails as an unreachable
/// service would, and nothing is loaded.
pub struct NoService;

impl ServiceAccess for NoService {
    fn connect(&self) -> Result<ServiceClient, ClientError> {
        Err(ClientError::NoStateDir(NoStateDir))
    }

    fn loaded_status(&self, _project: &ProjectRoot) -> Result<Option<ProjectStatus>, ClientError> {
        Ok(None)
    }

    fn running_notices(&self, _project: &ProjectRoot) -> Result<Option<Vec<Notice>>, ClientError> {
        Ok(None)
    }

    fn status(&self) -> Result<ServiceStatus, ClientError> {
        Err(ClientError::NoStateDir(NoStateDir))
    }

    fn stop(&self) -> Result<bool, ClientError> {
        Ok(false)
    }

    fn notify(&self, _request: &ServiceRequest) {}
}

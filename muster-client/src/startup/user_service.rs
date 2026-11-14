use muster_domain::value::ProjectRoot;
use muster_protocol::service::{Notice, ProjectStatus, ServiceRequest};

use crate::{
    connect::{asked::Asked, service_access::ServiceAccess, service_client::ServiceClient},
    failure::client_error::ClientError,
    startup::service_status::ServiceStatus,
};

/// This user's service, under the platform state directory.
#[derive(Debug, Clone, Copy, Default)]
pub struct UserService;

impl ServiceAccess for UserService {
    fn connect(&self) -> Result<ServiceClient, ClientError> {
        ServiceClient::connect()
    }

    fn loaded_status(&self, project: &ProjectRoot) -> Result<Option<ProjectStatus>, ClientError> {
        ServiceClient::loaded_status(project)
    }

    fn running_notices(&self, project: &ProjectRoot) -> Result<Option<Vec<Notice>>, ClientError> {
        ServiceClient::running_notices(project)
    }

    fn status(&self) -> Result<ServiceStatus, ClientError> {
        ServiceClient::status()
    }

    fn stop(&self) -> Result<bool, ClientError> {
        ServiceClient::stop()
    }

    fn notify(&self, request: &ServiceRequest) {
        ServiceClient::notify_if_running(request);
    }

    fn ask_if_running(&self, request: &ServiceRequest) -> Result<Asked, ClientError> {
        ServiceClient::ask_if_running(request)
    }
}

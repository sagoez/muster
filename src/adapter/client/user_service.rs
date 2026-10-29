use super::{
    client_error::ClientError, service_access::ServiceAccess, service_client::ServiceClient,
};
use crate::{adapter::service::ProjectStatus, domain::value::ProjectRoot};

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
}

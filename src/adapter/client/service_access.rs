use super::{client_error::ClientError, service_client::ServiceClient};
use crate::{adapter::service::ProjectStatus, domain::value::ProjectRoot};

/// How a driver reaches the muster service, injected so a test can serve
/// without ever touching the user's real service socket.
pub trait ServiceAccess: Send + Sync {
    /// A client of the service, starting or replacing it as needed.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when the service cannot be reached.
    fn connect(&self) -> Result<ServiceClient, ClientError>;

    /// The status of `project` when a running service has it loaded,
    /// without starting the service.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when a service holds the socket but does
    /// not answer.
    fn loaded_status(&self, project: &ProjectRoot) -> Result<Option<ProjectStatus>, ClientError>;
}

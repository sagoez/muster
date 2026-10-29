use super::{
    client_error::ClientError, service_client::ServiceClient, service_status::ServiceStatus,
};
use crate::{
    adapter::service::{ProjectStatus, ServiceRequest},
    domain::value::ProjectRoot,
};

/// Driven port: how a driver reaches the muster service, injected so a test
/// runs without ever touching the user's real service socket.
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

    /// Whether the service runs, its version and the projects it has
    /// loaded, without starting or replacing it.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when a service holds the socket but does
    /// not answer.
    fn status(&self) -> Result<ServiceStatus, ClientError>;

    /// Asks the service to exit, leaving every process running. Returns
    /// whether a service was running.
    ///
    /// # Errors
    /// Returns a [`ClientError`] when the service does not exit in time.
    fn stop(&self) -> Result<bool, ClientError>;

    /// Sends `request` to a service that is running, best effort, never
    /// starting one.
    fn notify(&self, request: &ServiceRequest);
}

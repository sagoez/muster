use super::service_client::ServiceClient;

/// Makes a client of a [`ServedService`](crate::startup::served_service::ServedService).
pub type Connector = Box<dyn Fn() -> ServiceClient + Send + Sync>;

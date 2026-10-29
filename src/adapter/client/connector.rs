use super::service_client::ServiceClient;

/// Makes a client of a [`ServedService`](super::served_service::ServedService).
pub(crate) type Connector = Box<dyn Fn() -> ServiceClient + Send + Sync>;

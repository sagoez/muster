use crossbeam_channel::Sender;

use super::service_response::ServiceResponse;

/// What handling one event leaves the service loop to do.
pub(super) enum Flow {
    /// Keep serving.
    Continue,
    /// Exit, then answer `reply` once the socket and lock are released.
    Exit(Sender<ServiceResponse>),
}

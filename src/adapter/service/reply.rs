use super::{service_response::ServiceResponse, supervisor::ScreenReply};

/// How a carried-out request is answered.
pub(super) enum Reply {
    /// With this response, now.
    Now(ServiceResponse),
    /// With a process's screen; one only a holder can serve is read off the
    /// loop first.
    Screen(ScreenReply),
    /// By a wait that holds the reply and answers it when it ends.
    Held,
    /// Once the service has exited.
    Exit,
}

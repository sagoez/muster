use super::status::Status;

/// What the gate decides for one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admission {
    /// The request names the view's token, and is served; this is its path
    /// below `/v/<token>`, starting with `/`.
    Admitted(String),
    /// The request is refused with this status.
    Refused(Status),
}

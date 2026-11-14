use super::status::Status;

/// What the gate decides for one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    /// The request carries the view's cookie and is served.
    Admitted,
    /// The first load, carrying the token as `key`: answered with the
    /// cookie and a redirect that drops the key from the address.
    GrantCookie,
    /// The request is refused with this status.
    Refused(Status),
}

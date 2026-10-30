use crate::adapter::service::ServiceResponse;

/// What asking a service that may be running made of a request, never
/// starting or replacing one.
#[derive(Debug, Clone, PartialEq)]
pub enum Asked {
    /// A service of this version answered.
    Answered(ServiceResponse),
    /// A service of another version runs; it was greeted, never asked.
    OtherVersion,
    /// No service answers, and another client holds the start lock while it
    /// starts one.
    Starting,
    /// No service answers, and none is being started.
    Absent,
}

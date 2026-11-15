use super::health::Health;

/// How a service of this version could not be asked for status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServiceDown {
    /// No service runs.
    Absent,
    /// Another client is starting the service.
    Starting,
    /// A service of another version runs.
    OtherVersion,
}

impl From<ServiceDown> for Health {
    /// The health the page shows for a service that is `down`.
    fn from(down: ServiceDown) -> Self {
        match down {
            ServiceDown::Absent => Self::Absent,
            ServiceDown::Starting => Self::Starting,
            ServiceDown::OtherVersion => Self::OtherVersion,
        }
    }
}

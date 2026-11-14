use muster_client::{Asked, ClientError};
use muster_protocol::service::{ProjectStatus, ServiceResponse};

/// What asking a running service for every loaded project's status found.
#[derive(Clone, Debug, PartialEq)]
pub enum StatusReading {
    /// A service of this version answered with each loaded project.
    Answered(Vec<ProjectStatus>),
    /// No service runs.
    Absent,
    /// Another client is starting the service.
    Starting,
    /// A service of another version runs.
    OtherVersion,
    /// The ask failed or got an answer that is no status, which says nothing
    /// of how the service stands.
    Failed,
}

impl From<Result<Asked, ClientError>> for StatusReading {
    fn from(asked: Result<Asked, ClientError>) -> Self {
        match asked {
            Ok(Asked::Answered(ServiceResponse::Status(projects))) => Self::Answered(projects),
            Ok(Asked::Answered(_)) | Err(_) => Self::Failed,
            Ok(Asked::OtherVersion(_)) => Self::OtherVersion,
            Ok(Asked::Starting) => Self::Starting,
            Ok(Asked::Absent) => Self::Absent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each way an ask ends reads as its status.
    #[test]
    fn each_ask_reads_as_its_status() {
        let read = |asked: Result<Asked, ClientError>| StatusReading::from(asked);

        assert_eq!(
            read(Ok(Asked::Answered(ServiceResponse::Status(Vec::new())))),
            StatusReading::Answered(Vec::new())
        );
        assert_eq!(
            read(Ok(Asked::Answered(ServiceResponse::Ok))),
            StatusReading::Failed
        );
        assert_eq!(read(Ok(Asked::Absent)), StatusReading::Absent);
        assert_eq!(read(Ok(Asked::Starting)), StatusReading::Starting);
        assert_eq!(
            read(Err(ClientError::UnexpectedAnswer("ok"))),
            StatusReading::Failed
        );
    }
}

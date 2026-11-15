use muster_client::{Asked, ClientError};
use muster_protocol::service::{ServiceResponse, StatusAnswer};

use super::ServiceDown;

/// What asking a running service for every loaded project's status found.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum StatusReading {
    /// A service of this version answered with each loaded project, and the
    /// newest event of its log the answer shows.
    Answered(StatusAnswer),
    /// No service of this version could be asked, and how it stands:
    /// absent, starting, or of another version.
    Down(ServiceDown),
    /// The ask failed or got an answer that is no status, which says nothing
    /// of how the service stands.
    Failed,
}

impl From<Result<Asked, ClientError>> for StatusReading {
    fn from(asked: Result<Asked, ClientError>) -> Self {
        match asked {
            Ok(Asked::Answered(ServiceResponse::Status(answer))) => Self::Answered(answer),
            Ok(Asked::Answered(_)) | Err(_) => Self::Failed,
            Ok(Asked::OtherVersion(_)) => Self::Down(ServiceDown::OtherVersion),
            Ok(Asked::Starting) => Self::Down(ServiceDown::Starting),
            Ok(Asked::Absent) => Self::Down(ServiceDown::Absent),
        }
    }
}

#[cfg(test)]
mod tests {
    use muster_protocol::service::EventSeq;

    use super::*;

    /// Each way an ask ends reads as its status, an answer keeping its
    /// cursor.
    #[test]
    fn each_ask_reads_as_its_status() {
        let read = |asked: Result<Asked, ClientError>| StatusReading::from(asked);
        let answer = StatusAnswer::builder()
            .projects(Vec::new())
            .cursor(EventSeq::builder().epoch(3).seq(14).build())
            .build();

        assert_eq!(
            read(Ok(Asked::Answered(ServiceResponse::Status(answer.clone())))),
            StatusReading::Answered(answer)
        );
        assert_eq!(
            read(Ok(Asked::Answered(ServiceResponse::Ok))),
            StatusReading::Failed
        );
        assert_eq!(
            read(Ok(Asked::Absent)),
            StatusReading::Down(ServiceDown::Absent)
        );
        assert_eq!(
            read(Ok(Asked::Starting)),
            StatusReading::Down(ServiceDown::Starting)
        );
        assert_eq!(
            read(Err(ClientError::UnexpectedAnswer("ok"))),
            StatusReading::Failed
        );
    }
}

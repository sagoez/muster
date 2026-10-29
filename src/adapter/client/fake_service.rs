use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use super::{
    client_error::ClientError, service_access::ServiceAccess, service_client::ServiceClient,
    service_status::ServiceStatus,
};
use crate::{
    adapter::{
        no_state_dir::NoStateDir,
        service::{Notice, ProjectStatus, ServiceRequest, ServiceResponse},
    },
    domain::value::ProjectRoot,
};

/// A service a test never reaches: connecting fails as an unreachable service
/// would and is counted, nothing is loaded, notices are answered as `running`
/// says, a request asked of a running service gets the answer the test
/// scripted, and every request it is told or asked is recorded.
#[derive(Default)]
pub(crate) struct FakeService {
    /// The live notices of a running service, or none when none runs.
    running: Option<Vec<Notice>>,
    /// What a request asked of a running service is answered, or none when
    /// no service runs to ask.
    answer: Option<ServiceResponse>,
    /// Every request the service was told, in order.
    told: Mutex<Vec<ServiceRequest>>,
    /// Every request the service was asked, in order.
    asked: Mutex<Vec<ServiceRequest>>,
    /// How many times a connection was attempted.
    connects: AtomicUsize,
}

impl FakeService {
    /// A service that runs and holds `notices`.
    pub(crate) fn running(notices: Vec<Notice>) -> Self {
        Self {
            running: Some(notices),
            ..Self::default()
        }
    }

    /// This service, running, answering every request asked of it with
    /// `answer`.
    pub(crate) fn answering(self, answer: ServiceResponse) -> Self {
        Self {
            answer: Some(answer),
            ..self
        }
    }

    /// Every request the service was told, in order.
    pub(crate) fn told(&self) -> Vec<ServiceRequest> {
        self.told.lock().unwrap().clone()
    }

    /// Every request the service was asked, in order.
    pub(crate) fn asked(&self) -> Vec<ServiceRequest> {
        self.asked.lock().unwrap().clone()
    }

    /// How many times a connection was attempted.
    pub(crate) fn connects(&self) -> usize {
        self.connects.load(Ordering::SeqCst)
    }
}

impl ServiceAccess for FakeService {
    fn connect(&self) -> Result<ServiceClient, ClientError> {
        self.connects.fetch_add(1, Ordering::SeqCst);
        Err(ClientError::NoStateDir(NoStateDir))
    }

    fn loaded_status(&self, _project: &ProjectRoot) -> Result<Option<ProjectStatus>, ClientError> {
        Ok(None)
    }

    fn running_notices(&self, _project: &ProjectRoot) -> Result<Option<Vec<Notice>>, ClientError> {
        Ok(self.running.clone())
    }

    fn status(&self) -> Result<ServiceStatus, ClientError> {
        Err(ClientError::NoStateDir(NoStateDir))
    }

    fn stop(&self) -> Result<bool, ClientError> {
        Ok(false)
    }

    fn notify(&self, request: &ServiceRequest) {
        self.told.lock().unwrap().push(request.clone());
    }

    fn ask_if_running(
        &self,
        request: &ServiceRequest,
    ) -> Result<Option<ServiceResponse>, ClientError> {
        self.asked.lock().unwrap().push(request.clone());
        Ok(self.answer.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake with no answer scripted runs no service; one answering gives
    /// its answer, and both record what they were asked.
    #[test]
    fn a_scripted_answer_is_given_and_every_ask_recorded() {
        let request = ServiceRequest::SettingsChanged;
        let absent = FakeService::default();
        let running = FakeService::default().answering(ServiceResponse::Ok);

        assert_eq!(absent.ask_if_running(&request).unwrap(), None);
        assert_eq!(
            running.ask_if_running(&request).unwrap(),
            Some(ServiceResponse::Ok)
        );
        assert_eq!(absent.asked(), vec![request.clone()]);
        assert_eq!(running.asked(), vec![request]);
        assert!(running.told().is_empty());
        assert_eq!(running.connects(), 0);
    }
}

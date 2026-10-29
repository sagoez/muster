use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use super::{
    asked::Asked,
    client_error::ClientError,
    connector::Connector,
    served_service::ServedService,
    service_access::ServiceAccess,
    service_client::ServiceClient,
    service_status::ServiceStatus,
};
use crate::{
    adapter::{
        no_state_dir::NoStateDir,
        service::{Notice, ProjectStatus, ServiceRequest, ServiceResponse},
    },
    domain::value::ProjectRoot,
};

/// A service a test never reaches unless told to: connecting fails as an
/// unreachable service would, or reaches the [`ServedService`] it was handed,
/// and is counted either way; nothing is loaded, notices are answered as
/// `running` says, a request asked of a service that may run gets what the
/// test scripted, and every request it is told or asked is recorded.
#[derive(Default)]
pub(crate) struct FakeService {
    /// What a connection reaches, or none when no service can be reached.
    starts: Option<Connector>,
    /// The live notices of a running service, or none when none runs.
    running: Option<Vec<Notice>>,
    /// What each ask is answered, in turn, the last one for every ask after
    /// it; none scripted answers that no service runs.
    asks: Mutex<Vec<Asked>>,
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
        self.asked_in_turn(vec![Asked::Answered(answer)])
    }

    /// This service, giving `asks` to the asks made of it in turn, the last
    /// to every ask after it.
    pub(crate) fn asked_in_turn(self, asks: Vec<Asked>) -> Self {
        Self {
            asks: Mutex::new(asks),
            ..self
        }
    }

    /// This service, connecting to `served` as if it had been started.
    pub(crate) fn starting(self, served: &ServedService) -> Self {
        Self {
            starts: Some(served.connector()),
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
        self.starts
            .as_ref()
            .map(|connect| connect())
            .ok_or(ClientError::NoStateDir(NoStateDir))
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

    fn ask_if_running(&self, request: &ServiceRequest) -> Result<Asked, ClientError> {
        self.asked.lock().unwrap().push(request.clone());
        let mut asks = self.asks.lock().unwrap();
        Ok(match asks.len() {
            0 => Asked::Absent,
            1 => asks[0].clone(),
            _ => asks.remove(0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake with nothing scripted runs no service; one answering gives its
    /// answer every time, one scripted in turn gives each then keeps the
    /// last, and all record what they were asked.
    #[test]
    fn scripted_asks_are_given_in_turn_and_every_ask_recorded() {
        let request = ServiceRequest::SettingsChanged;
        let absent = FakeService::default();
        let running = FakeService::default().answering(ServiceResponse::Ok);
        let starting = FakeService::default().asked_in_turn(vec![Asked::Starting, Asked::Absent]);

        assert_eq!(absent.ask_if_running(&request).unwrap(), Asked::Absent);
        for _ in 0..2 {
            assert_eq!(
                running.ask_if_running(&request).unwrap(),
                Asked::Answered(ServiceResponse::Ok)
            );
        }
        for expected in [Asked::Starting, Asked::Absent, Asked::Absent] {
            assert_eq!(starting.ask_if_running(&request).unwrap(), expected);
        }
        assert_eq!(absent.asked(), vec![request.clone()]);
        assert_eq!(running.asked(), vec![request.clone(), request]);
        assert!(running.told().is_empty());
        assert_eq!(running.connects(), 0);
    }
}

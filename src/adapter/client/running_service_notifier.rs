use std::sync::Arc;

use super::service_access::ServiceAccess;
use crate::{
    adapter::service::ServiceRequest,
    domain::{port::StateChangeNotifier, value::ProjectRoot},
};

/// Tells the service `service` reaches that stored state changed, when one
/// runs. A service that is absent, busy or another version is left alone: it
/// reads fresh state whenever it next loads a project.
#[derive(Clone)]
pub struct RunningServiceNotifier {
    /// The service told.
    service: Arc<dyn ServiceAccess>,
}

impl RunningServiceNotifier {
    /// The notifier telling the service `service` reaches.
    pub fn new(service: Arc<dyn ServiceAccess>) -> Self {
        Self { service }
    }
}

impl StateChangeNotifier for RunningServiceNotifier {
    fn stack_changed(&self, root: &ProjectRoot) {
        self.service.notify(&ServiceRequest::StackChanged {
            project: root.clone(),
        });
    }

    fn settings_changed(&self) {
        self.service.notify(&ServiceRequest::SettingsChanged);
    }
}

use super::service_client::ServiceClient;
use crate::{
    adapter::service::ServiceRequest,
    domain::{config::StackEdit, port::StateChangeNotifier, value::ProjectRoot},
};

/// Tells this user's service that stored state changed, when one runs. A
/// service that is absent, busy or another version is left alone: it reads
/// fresh state whenever it next loads a project.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunningServiceNotifier;

impl StateChangeNotifier for RunningServiceNotifier {
    fn stack_changed(&self, root: &ProjectRoot) {
        ServiceClient::notify_if_running(&ServiceRequest::StackChanged {
            project: root.clone(),
        });
    }

    fn stack_edited(&self, root: &ProjectRoot, edit: &StackEdit) {
        ServiceClient::notify_if_running(&ServiceRequest::StackEdited {
            project: root.clone(),
            process: edit.process().clone(),
            message: edit.to_string(),
        });
    }

    fn settings_changed(&self) {
        ServiceClient::notify_if_running(&ServiceRequest::SettingsChanged);
    }
}

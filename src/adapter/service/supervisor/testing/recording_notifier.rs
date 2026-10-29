use std::{cell::RefCell, rc::Rc};

use crate::domain::{notification::Notification, port::Notifier};

/// A notifier that records every notification delivered.
#[derive(Clone, Default)]
pub(crate) struct RecordingNotifier {
    delivered: Rc<RefCell<Vec<Notification>>>,
}

impl RecordingNotifier {
    /// Every notification delivered, oldest first.
    pub(crate) fn delivered(&self) -> Vec<Notification> {
        self.delivered.borrow().clone()
    }
}

impl Notifier for RecordingNotifier {
    fn notify(&self, notification: &Notification) {
        self.delivered.borrow_mut().push(notification.clone());
    }
}

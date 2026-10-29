use std::sync::Mutex;

use crate::domain::{port::StateChangeNotifier, value::ProjectRoot};

/// A [`StateChangeNotifier`] that records what it was told.
#[derive(Default)]
pub struct RecordingNotifier {
    /// Each project whose stack changed, in order.
    pub stacks: Mutex<Vec<ProjectRoot>>,
    /// How many times the settings changed.
    pub settings: Mutex<usize>,
}

impl StateChangeNotifier for RecordingNotifier {
    fn stack_changed(&self, root: &ProjectRoot) {
        self.stacks.lock().unwrap().push(root.clone());
    }

    fn settings_changed(&self) {
        *self.settings.lock().unwrap() += 1;
    }
}

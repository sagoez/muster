use std::sync::Mutex;

use getset::{Getters, MutGetters};

use crate::domain::{port::StateChangeNotifier, value::ProjectRoot};

/// A [`StateChangeNotifier`] that records what it was told.
#[derive(Default, Getters, MutGetters)]
pub(crate) struct RecordingNotifier {
    /// Each folder registered as a project, in order.
    #[getset(get = "pub(crate)")]
    registered: Mutex<Vec<ProjectRoot>>,
    /// Each project whose stack changed, in order.
    #[getset(get = "pub(crate)", get_mut = "pub(crate)")]
    stacks: Mutex<Vec<ProjectRoot>>,
    /// How many times the settings changed.
    #[getset(get = "pub(crate)", get_mut = "pub(crate)")]
    settings: Mutex<usize>,
}

impl StateChangeNotifier for RecordingNotifier {
    fn project_registered(&self, root: &ProjectRoot) {
        self.registered.lock().unwrap().push(root.clone());
    }

    fn stack_changed(&self, root: &ProjectRoot) {
        self.stacks.lock().unwrap().push(root.clone());
    }

    fn settings_changed(&self) {
        *self.settings.lock().unwrap() += 1;
    }
}

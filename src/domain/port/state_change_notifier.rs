use crate::domain::{config::StackEdit, value::ProjectRoot};

/// Tells whoever runs the stack that stored state changed. Best effort.
pub trait StateChangeNotifier {
    /// The stack of the project at `root` changed.
    fn stack_changed(&self, root: &ProjectRoot);

    /// An agent made `edit` to the stack of the project at `root`, which the
    /// user is to be told of.
    fn stack_edited(&self, root: &ProjectRoot, edit: &StackEdit);

    /// The settings changed.
    fn settings_changed(&self);
}

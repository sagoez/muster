use crate::domain::value::ProjectRoot;

/// Tells whoever runs the stack that stored state changed. Best effort.
pub trait StateChangeNotifier {
    /// The stack of the project at `root` changed.
    fn stack_changed(&self, root: &ProjectRoot);

    /// The settings changed.
    fn settings_changed(&self);
}

use crate::domain::value::ProjectRoot;

/// Tells whoever runs the stack that stored state changed. Best effort.
pub trait StateChangeNotifier {
    /// The folder `root` was just registered as a project, before its
    /// stack's change is told.
    fn project_registered(&self, root: &ProjectRoot);

    /// The stack of the project at `root` changed.
    fn stack_changed(&self, root: &ProjectRoot);

    /// The settings changed.
    fn settings_changed(&self);
}

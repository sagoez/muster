use crate::domain::notification::Notification;

/// Delivers a [`Notification`] to the user out of band, for example as an OS
/// desktop notification; notices are kept by the service itself. Called only
/// from the runtime loop, so it carries no thread-safety bound.
pub trait Notifier {
    /// Shows `notification` to the user. Best effort: a delivery failure is
    /// swallowed, so a missing notification daemon never disrupts the workspace.
    fn notify(&self, notification: &Notification);
}

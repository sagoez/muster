use crate::domain::{
    notification::{Notification, NotificationId, NotificationScope},
    value::PaneId,
};

/// Delivers a [`Notification`] to the user outside the TUI, for example as an OS
/// desktop notification. The in-app status line is handled by the TUI itself, so
/// this port covers only the out-of-band channel. Called only from the runtime
/// loop, so it carries no thread-safety bound.
pub trait Notifier {
    /// Shows `notification` to the user. Best effort: a delivery failure is
    /// swallowed, so a missing notification daemon never disrupts the workspace.
    fn notify(&self, notification: &Notification);

    /// Closes the previously delivered Kitty notification from one pane lifetime
    /// with `identifier`. Adapters without close support may ignore the request.
    fn close(&self, _pane: PaneId, _scope: NotificationScope, _identifier: &NotificationId) {}
}

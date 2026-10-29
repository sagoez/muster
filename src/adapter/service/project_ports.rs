use crate::domain::port::Notifier;

/// Builds the driven adapters each project's supervisor runs on, as the
/// service loads that project. An adapter that is absent is not wired.
pub trait ProjectPorts {
    /// Desktop notifications.
    fn notifier(&self) -> Option<Box<dyn Notifier>>;
}

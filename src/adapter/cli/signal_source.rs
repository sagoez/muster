use std::sync::{Arc, mpsc::SyncSender};

use super::{attach_event::AttachEvent, error::CliError, session_alarm::SessionAlarm};

/// Where an attach session hears that its terminal resized or that a signal
/// ends it.
pub trait SignalSource: Send {
    /// Starts posting resizes and ending signals to `events`, never waiting
    /// on it: an ending signal is also recorded in `alarm`, so a full inbox
    /// never loses it.
    ///
    /// # Errors
    /// Returns a [`CliError`] when the signals cannot be listened for.
    fn listen(
        &mut self,
        events: SyncSender<AttachEvent>,
        alarm: Arc<SessionAlarm>,
    ) -> Result<(), CliError>;

    /// Stops posting. An ending signal that arrives afterwards still ends
    /// the process as it would without a session.
    fn stop(&mut self);
}

use super::holder_screen::HolderScreen;

/// What the supervisor makes of a request for a process's screen.
#[derive(Debug)]
pub enum ScreenReply {
    /// The screen, ready to send.
    Ready(String),
    /// A screen its holder must be asked for. The caller reads it off the
    /// supervisor's loop and settles it with
    /// [`Supervisor::settle_holder_read`](super::Supervisor::settle_holder_read).
    ReadHolder(HolderScreen),
}

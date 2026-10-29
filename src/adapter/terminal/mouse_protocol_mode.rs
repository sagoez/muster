/// xterm mouse-reporting mode a child has requested, mapped from the terminal's
/// private modes so pointer encoding does not depend on the emulator backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseProtocolMode {
    /// No mouse reporting.
    None,
    /// Report presses and releases (mode 1000).
    PressRelease,
    /// Report presses, releases, and motion while a button is held (mode 1002).
    ButtonMotion,
    /// Report presses, releases, and all motion (mode 1003).
    AnyMotion,
}

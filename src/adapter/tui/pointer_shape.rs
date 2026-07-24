use strum::Display;

/// Pointer shapes muster asks the host terminal to show (xterm OSC 22),
/// restoring the I-beam terminals hide once an app captures the mouse.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Display)]
pub enum PointerShape {
    /// The regular arrow pointer.
    #[default]
    #[strum(serialize = "default")]
    Default,
    /// The text-selection I-beam shown over selectable pane text.
    #[strum(serialize = "text")]
    Text,
}

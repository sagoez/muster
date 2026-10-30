use strum::FromRepr;

/// What a frame carries, as its one-byte wire tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromRepr)]
#[repr(u8)]
pub enum FrameTag {
    /// A client introduces itself and its role.
    Hello = 1,
    /// Bytes a client writes to the child.
    Input = 2,
    /// A client's terminal size.
    Resize = 3,
    /// A supervisor's process control request.
    Control = 4,
    /// A supervisor acknowledges the held exit outcome.
    CollectExit = 5,
    /// The rebuilt screen sent to a client when it says hello.
    Snapshot = 6,
    /// Bytes the child wrote.
    Output = 7,
    /// The child's exit outcome.
    Exited = 8,
    /// The holder refused a client message.
    Rejected = 9,
    /// The holder hit a fault it recovered from, for the user to know.
    Warning = 10,
    /// The writable attach clients now connected.
    Attached = 11,
}

use super::ending_signal::EndingSignal;
use crate::adapter::holder::{HolderError, HolderMessage};

/// Something an attach session reacts to, from the terminal, the holder or
/// a signal, delivered on the session's one channel.
#[derive(Debug)]
pub enum AttachEvent {
    /// Bytes the person typed.
    Typed(Vec<u8>),
    /// The terminal's input ended.
    InputClosed,
    /// A message from the holder.
    Holder(HolderMessage),
    /// The holder connection broke.
    HolderLost(HolderError),
    /// A helper thread of the session panicked, so the session cannot go on.
    HelperFailed,
    /// The terminal changed size.
    Resized,
    /// A signal ended the client.
    Ended(EndingSignal),
}

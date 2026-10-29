use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

/// What a [`DetachScanner`](super::DetachScanner) made of one chunk of typed
/// input.
#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct ScanResult {
    /// The bytes to pass on to the process, in the order they were typed.
    #[getset(get = "pub")]
    forward: Vec<u8>,
    /// Whether the detach key was pressed; nothing typed after it is passed
    /// on.
    #[getset(get_copy = "pub")]
    detached: bool,
}

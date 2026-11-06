use getset::Getters;
use muster_domain::value::OutputCursor;
use typed_builder::TypedBuilder;

/// The last bytes of a process's log, from a line boundary, for a service
/// that never saw the process run.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct LogTail {
    /// Where the bytes start in the output.
    at: OutputCursor,
    /// The bytes, up to where the log ends.
    bytes: Vec<u8>,
}

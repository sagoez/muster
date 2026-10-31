use getset::Getters;
use typed_builder::TypedBuilder;

use crate::{adapter::terminal::stream_marks::StreamMarks, domain::value::OutputCursor};

/// The last bytes of a process's log and what a reader needs to render them
/// as every other reader does, for a service that never saw the process run.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct LogTail {
    /// Where the bytes start in the output.
    at: OutputCursor,
    /// The bytes, up to where the log ends.
    bytes: Vec<u8>,
    /// The log's marks: its run starts and where the screen changed.
    marks: StreamMarks,
}

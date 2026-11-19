use getset::CopyGetters;
use serde::Serialize;
use typed_builder::TypedBuilder;

/// What `process_send` answers once the text reached the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct SentView {
    /// Whether the text was typed into the terminal.
    sent: bool,
}

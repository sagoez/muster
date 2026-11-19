use getset::Getters;
use muster_domain::value::ProcessName;
use serde::Serialize;
use typed_builder::TypedBuilder;

/// What `process_screen` answers: what a process's terminal shows now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ScreenView {
    /// The process.
    process: ProcessName,
    /// Its terminal's screen, as text.
    screen: String,
}

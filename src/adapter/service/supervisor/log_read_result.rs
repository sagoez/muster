use std::{collections::HashMap, mem};

use getset::Getters;
use typed_builder::TypedBuilder;

use super::log_answer::LogAnswer;
use crate::domain::value::OutputCursor;

/// What one read of a process's log off the loop brought back.
#[derive(Debug, Clone, Default, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct LogReadResult {
    /// The log's last bytes, with where they start, which end where the log
    /// does; none when the end was not asked or could not be read.
    #[builder(default)]
    tail: Option<(OutputCursor, Vec<u8>)>,
    /// What was found for each thing asked, by the key of the held wait or
    /// answer that asked it; one missing could not be read.
    #[builder(default)]
    answers: HashMap<u64, LogAnswer>,
}

impl LogReadResult {
    /// Takes the tail, leaving none.
    pub fn take_tail(&mut self) -> Option<(OutputCursor, Vec<u8>)> {
        self.tail.take()
    }

    /// Takes the answers, leaving none.
    pub fn take_answers(&mut self) -> HashMap<u64, LogAnswer> {
        mem::take(&mut self.answers)
    }
}

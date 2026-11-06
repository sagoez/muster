use thiserror::Error;

/// Why a label names no author: it is blank once trimmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("author label must not be empty")]
pub struct BlankAuthor;

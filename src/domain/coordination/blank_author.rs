use thiserror::Error;

/// A blank author label, which names nobody.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("author label must not be empty")]
pub struct BlankAuthor;

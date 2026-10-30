use std::{error::Error, fmt};

use super::wrapped::Wrapped;

/// An error whose display already embeds its source.
#[derive(Debug)]
pub(super) struct Embedding(Wrapped);

impl Embedding {
    /// An error embedding `source`'s display in its own.
    pub(super) fn new(source: Wrapped) -> Self {
        Self(source)
    }
}

impl fmt::Display for Embedding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not open: {}", self.0)
    }
}

impl Error for Embedding {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

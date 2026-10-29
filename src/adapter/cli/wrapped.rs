use std::{error::Error, fmt};

/// A source error.
#[derive(Debug)]
pub(super) struct Wrapped;

impl fmt::Display for Wrapped {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("locked")
    }
}

impl Error for Wrapped {}

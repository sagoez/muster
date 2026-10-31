use std::{fmt, str::FromStr};

use getset::CopyGetters;
use typed_builder::TypedBuilder;

use super::mark_kind::MarkKind;
use crate::domain::value::OutputCursor;

/// Separates a mark's kind from its offset in its text form.
const FIELD_SEPARATOR: char = ' ';

/// A point in a process's output where the terminal state a reader starting
/// there needs changed: a run start, or the alternate screen coming up or
/// going away. Its text form is `<kind> <offset>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct StreamMark {
    /// Where in the output: the state holds for the byte here and after.
    at: OutputCursor,
    /// What changed there.
    kind: MarkKind,
}

impl fmt::Display for StreamMark {
    /// Writes the mark as `<kind> <offset>`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}{FIELD_SEPARATOR}{}",
            self.kind,
            self.at.offset()
        )
    }
}

impl FromStr for StreamMark {
    type Err = strum::ParseError;

    /// Reads a mark written by [`fmt::Display`].
    ///
    /// # Errors
    /// Returns a [`strum::ParseError`] for text of another form.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let (kind, offset) = raw
            .trim()
            .split_once(FIELD_SEPARATOR)
            .ok_or(strum::ParseError::VariantNotFound)?;
        let offset = offset
            .parse()
            .map_err(|_| strum::ParseError::VariantNotFound)?;
        Ok(Self {
            at: OutputCursor::new(offset),
            kind: kind.parse()?,
        })
    }
}

use std::fmt;

use getset::CopyGetters;
use typed_builder::TypedBuilder;

use super::PtyError;
use crate::domain::value::{Cols, Rows, ValueParseError};

/// Separates rows from columns in a written size.
const SIZE_SEPARATOR: char = 'x';
/// The form a size is written in.
const SIZE_FORM: &str = "ROWSxCOLS such as 24x80";

/// Dimensions of a pseudo-terminal, in character cells. Its [`Rows`] and
/// [`Cols`] keep every size within the PTY bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct PtySize {
    rows: Rows,
    cols: Cols,
}

impl PtySize {
    /// The size of `rows` by `cols`.
    ///
    /// # Errors
    /// Returns [`PtyError::SizeOutOfBounds`] when either is outside the PTY
    /// bounds.
    pub fn try_new(rows: u16, cols: u16) -> Result<Self, PtyError> {
        match (Rows::try_new(rows), Cols::try_new(cols)) {
            (Ok(rows), Ok(cols)) => Ok(Self { rows, cols }),
            _ => Err(PtyError::SizeOutOfBounds { rows, cols }),
        }
    }

    /// Reads `ROWSxCOLS`, such as `24x80`, as two counts, leaving the PTY
    /// bounds to [`Self::try_new`].
    ///
    /// # Errors
    /// Returns [`ValueParseError`] when the text is not two counts around `x`.
    pub fn dimensions(raw: &str) -> Result<(u16, u16), ValueParseError> {
        raw.trim()
            .split_once(SIZE_SEPARATOR)
            .and_then(|(rows, cols)| Some((rows.parse().ok()?, cols.parse().ok()?)))
            .ok_or_else(|| {
                ValueParseError::builder()
                    .expected(SIZE_FORM)
                    .found(raw)
                    .build()
            })
    }
}

impl fmt::Display for PtySize {
    /// Writes the size as `ROWSxCOLS`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}{SIZE_SEPARATOR}{}", self.rows, self.cols)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value::{MAX_COLS, MAX_ROWS, MIN_COLS, MIN_ROWS};

    #[test]
    fn reads_and_writes_rows_by_columns() {
        assert_eq!(PtySize::dimensions("24x80").unwrap(), (24, 80));
        assert_eq!(PtySize::try_new(24, 80).unwrap().to_string(), "24x80");
        for refused in ["24", "24x", "x80", "24 by 80", "-1x80"] {
            assert!(PtySize::dimensions(refused).is_err(), "{refused}");
        }
    }

    /// A size outside the bounds cannot be built, and the refusal names it.
    #[test]
    fn refuses_a_size_outside_the_bounds() {
        assert!(PtySize::try_new(MIN_ROWS - 1, 80).is_err());
        assert!(PtySize::try_new(24, MIN_COLS - 1).is_err());
        assert!(PtySize::try_new(MAX_ROWS + 1, 80).is_err());
        assert!(
            PtySize::try_new(24, MAX_COLS + 1)
                .unwrap_err()
                .to_string()
                .contains("24x2001")
        );
        assert_eq!(
            PtySize::try_new(MAX_ROWS, MAX_COLS).unwrap().to_string(),
            "1000x2000"
        );
    }
}

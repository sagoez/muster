use std::{fmt, str::FromStr};

use getset::CopyGetters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::PtyError;
use crate::domain::value::{Cols, Rows, ValueParseError};

/// Separates rows from columns in a written size.
const SIZE_SEPARATOR: char = 'x';
/// The form a size is written in.
const SIZE_FORM: &str = "ROWSxCOLS such as 24x80";

/// Dimensions of a pseudo-terminal, in character cells. Every PTY a holder
/// runs stays within [`PtySize::MIN_ROWS`] by [`PtySize::MIN_COLS`] to
/// [`PtySize::MAX_ROWS`] by [`PtySize::MAX_COLS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, CopyGetters, TypedBuilder)]
#[serde(rename_all = "kebab-case")]
#[getset(get_copy = "pub")]
pub struct PtySize {
    rows: Rows,
    cols: Cols,
}

impl PtySize {
    /// The most columns a PTY may have, bounding the screen a holder keeps.
    pub const MAX_COLS: u16 = 2000;
    /// The most rows a PTY may have, bounding the screen a holder keeps.
    pub const MAX_ROWS: u16 = 1000;
    /// The fewest columns a PTY may have: a wide character needs two.
    pub const MIN_COLS: u16 = 2;
    /// The fewest rows a PTY may have.
    pub const MIN_ROWS: u16 = 1;

    /// Whether both dimensions are within the PTY bounds.
    pub fn is_within_bounds(self) -> bool {
        (Self::MIN_ROWS..=Self::MAX_ROWS).contains(&self.rows.into_inner())
            && (Self::MIN_COLS..=Self::MAX_COLS).contains(&self.cols.into_inner())
    }

    /// This size, or the nearest one within the PTY bounds.
    pub fn clamped(self) -> Self {
        Self {
            rows: Rows::new(self.rows.into_inner().clamp(Self::MIN_ROWS, Self::MAX_ROWS)),
            cols: Cols::new(self.cols.into_inner().clamp(Self::MIN_COLS, Self::MAX_COLS)),
        }
    }

    /// This size when it is within the PTY bounds.
    ///
    /// # Errors
    /// Returns [`PtyError::SizeOutOfBounds`] when either dimension is outside
    /// them.
    pub fn checked(self) -> Result<Self, PtyError> {
        if self.is_within_bounds() {
            Ok(self)
        } else {
            Err(PtyError::SizeOutOfBounds {
                rows: self.rows.into_inner(),
                cols: self.cols.into_inner(),
            })
        }
    }
}

impl FromStr for PtySize {
    type Err = ValueParseError;

    /// Reads `ROWSxCOLS`, such as `24x80`, without checking the PTY bounds.
    ///
    /// # Errors
    /// Returns [`ValueParseError`] when the text is not two counts around `x`.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        raw.trim()
            .split_once(SIZE_SEPARATOR)
            .and_then(|(rows, cols)| Some((rows.parse().ok()?, cols.parse().ok()?)))
            .map(|(rows, cols)| {
                Self::builder()
                    .rows(Rows::new(rows))
                    .cols(Cols::new(cols))
                    .build()
            })
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

    #[test]
    fn reads_and_writes_rows_by_columns() {
        let parsed: PtySize = "24x80".parse().unwrap();

        assert_eq!(parsed, size(24, 80));
        assert_eq!(parsed.to_string(), "24x80");
        for refused in ["24", "24x", "x80", "24 by 80", "-1x80"] {
            assert!(refused.parse::<PtySize>().is_err(), "{refused}");
        }
    }

    fn size(rows: u16, cols: u16) -> PtySize {
        PtySize::builder()
            .rows(Rows::new(rows))
            .cols(Cols::new(cols))
            .build()
    }

    /// A size outside the bounds is pulled to the nearest edge.
    #[test]
    fn clamping_pulls_a_size_into_the_bounds() {
        assert_eq!(
            size(0, 1).clamped(),
            size(PtySize::MIN_ROWS, PtySize::MIN_COLS)
        );
        assert_eq!(
            size(u16::MAX, u16::MAX).clamped(),
            size(PtySize::MAX_ROWS, PtySize::MAX_COLS)
        );
        assert_eq!(size(24, 80).clamped(), size(24, 80));
    }

    /// A size outside the bounds fails the check, naming itself.
    #[test]
    fn checking_refuses_a_size_outside_the_bounds() {
        assert!(size(0, 80).checked().is_err());
        assert!(size(24, 1).checked().is_err());
        assert!(size(1001, 80).checked().is_err());
        assert!(
            size(24, 2001)
                .checked()
                .unwrap_err()
                .to_string()
                .contains("24x2001")
        );
        assert_eq!(size(24, 80).checked().unwrap(), size(24, 80));
    }
}

use strum::{Display, EnumIter, EnumString, IntoStaticStr};

use crate::domain::value::{Cols, Rows};

/// A terminal cell is about this many times as tall as it is wide.
const CELL_ASPECT: u32 = 2;
/// The narrowest a side-by-side half may be, in columns.
const MIN_HALF_COLS: u32 = 40;
/// The halves a side-by-side split makes.
const HALVES: u32 = 2;

/// Which side of the caller's pane a new pane opens on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString, EnumIter, IntoStaticStr)]
#[strum(serialize_all = "kebab-case")]
pub enum SplitDirection {
    /// Beside it, to the right.
    Right,
    /// Below it.
    Down,
}

impl SplitDirection {
    /// The split for a pane whose size cannot be learned.
    pub const FOR_UNKNOWN_AREA: Self = Self::Down;

    /// The split for a pane `cols` wide and `rows` tall: right when it looks
    /// wider than tall and halves into two columns of at least
    /// `MIN_HALF_COLS`, down otherwise.
    pub fn for_area(cols: Cols, rows: Rows) -> Self {
        let (cols, rows) = (u32::from(cols.into_inner()), u32::from(rows.into_inner()));
        if cols >= rows * CELL_ASPECT && cols >= MIN_HALF_COLS * HALVES {
            Self::Right
        } else {
            Self::Down
        }
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    /// The split for a pane `cols` by `rows`.
    fn split(cols: u16, rows: u16) -> SplitDirection {
        SplitDirection::for_area(Cols::try_new(cols).unwrap(), Rows::try_new(rows).unwrap())
    }

    /// A pane wider than tall in cells splits right; one that is not splits
    /// down.
    #[test]
    fn a_wide_pane_splits_right_and_a_tall_one_down() {
        assert_eq!(split(200, 50), SplitDirection::Right);
        assert_eq!(split(100, 50), SplitDirection::Right);
        assert_eq!(split(99, 50), SplitDirection::Down);
        assert_eq!(split(80, 50), SplitDirection::Down);
    }

    /// A wide pane whose halves would be narrower than the minimum splits
    /// down.
    #[test]
    fn a_pane_too_narrow_to_halve_splits_down() {
        assert_eq!(split(79, 10), SplitDirection::Down);
        assert_eq!(split(80, 10), SplitDirection::Right);
    }

    /// Each direction prints in kebab case and reads back from it.
    #[test]
    fn split_direction_reads_and_prints_kebab_case() {
        let printed: Vec<String> = SplitDirection::iter().map(|d| d.to_string()).collect();
        assert_eq!(printed, ["right", "down"]);
        for direction in SplitDirection::iter() {
            assert_eq!(
                direction.to_string().parse::<SplitDirection>(),
                Ok(direction)
            );
            assert_eq!(<&str>::from(direction), direction.to_string());
        }
        assert!("Right".parse::<SplitDirection>().is_err());
    }
}

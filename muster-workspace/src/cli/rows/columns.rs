use getset::CopyGetters;
use typed_builder::TypedBuilder;
use unicode_width::UnicodeWidthStr;

use super::row::Row;

/// The display widths a set of rows pads its labels and words to, so names,
/// state words and facts line up the same way in every command.
#[derive(Debug, Clone, Copy, Default, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct Columns {
    /// The widest label.
    label: usize,
    /// The widest word.
    word: usize,
}

impl Columns {
    /// The widths of `rows`' widest label and word.
    pub fn of(rows: &[Row]) -> Self {
        let widest = |text: Option<&String>| text.map_or(0, |text| text.as_str().width());
        Self {
            label: rows
                .iter()
                .map(|row| widest(row.label().as_ref()))
                .max()
                .unwrap_or(0),
            word: rows
                .iter()
                .map(|row| widest(row.word().as_ref()))
                .max()
                .unwrap_or(0),
        }
    }
}

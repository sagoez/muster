//! A row a statement writes, giving each of its table's columns a value.

use rusqlite::types::Value;

use super::column_set::Column;

/// A row of the table whose columns are `C`. Its [`RowSource::value`] is an
/// exhaustive `match`, so a column added to `C` does not compile until the
/// row gives it a value, and no statement can leave one unbound.
pub(crate) trait RowSource<C: Column> {
    /// The value `column` takes in this row.
    fn value(&self, column: C) -> Value;
}

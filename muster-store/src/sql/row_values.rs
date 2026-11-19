//! The values of one row a statement writes, each bound by its column's
//! name rather than by its place in a list.

use getset::Getters;
use rusqlite::ToSql;
use typed_builder::TypedBuilder;

use super::{
    bound_value::BoundValue,
    column_set::{Column, ColumnSet},
    row_source::RowSource,
};

/// Values to bind, each to the named parameter of its column.
#[derive(Clone, Debug, PartialEq, Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct RowValues {
    /// The values, in the order of their columns.
    bound: Vec<BoundValue>,
}

impl RowValues {
    /// The value `row` gives each of `columns`, so a statement listing
    /// `columns` binds every parameter it names, and only those.
    pub(crate) fn of<C: Column>(columns: &ColumnSet<C>, row: &impl RowSource<C>) -> Self {
        Self::builder()
            .bound(
                columns
                    .columns()
                    .iter()
                    .map(|column| {
                        BoundValue::builder()
                            .param(column.param())
                            .value(row.value(*column))
                            .build()
                    })
                    .collect(),
            )
            .build()
    }

    /// The values as rusqlite binds named parameters.
    pub(crate) fn params(&self) -> Vec<(&str, &dyn ToSql)> {
        self.bound
            .iter()
            .map(|bound| (bound.param().as_str(), bound.value() as &dyn ToSql))
            .collect()
    }
}

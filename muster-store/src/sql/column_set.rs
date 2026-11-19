//! Columns of a table written into a statement, each bound by its own name.

use getset::Getters;
use typed_builder::TypedBuilder;

use super::{column_form::ColumnForm, sql_list::SqlList};

/// A column of a table, named as the schema names it.
pub(crate) trait Column: Copy + AsRef<str> {}

/// Columns in the order a statement lists them.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct ColumnSet<C> {
    /// The columns.
    columns: Vec<C>,
}

impl<C: Column> ColumnSet<C> {
    /// `columns`, in their order.
    pub(crate) fn of(columns: impl IntoIterator<Item = C>) -> Self {
        Self::builder()
            .columns(columns.into_iter().collect())
            .build()
    }

    /// Each column written in `form`, joined as `list`: `a, b` for a
    /// `SELECT`, `a = :a AND b = :b` for a `WHERE`.
    pub(crate) fn written(&self, form: ColumnForm, list: SqlList) -> String {
        let entries: Vec<String> = self
            .columns
            .iter()
            .map(|column| form.of(column.as_ref()))
            .collect();
        list.join(&entries)
    }
}

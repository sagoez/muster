//! Columns of a table written into a statement, each bound by its own name.

use getset::Getters;
use muster_domain::text::{ListForm, ValueList};
use typed_builder::TypedBuilder;

/// Opens a named parameter.
const PARAM_MARK: &str = ":";
/// Joins a column to the parameter it is set to or compared with.
const EQUALS: &str = " = ";
/// Joins the conditions of a `WHERE`.
const AND: &str = " AND ";

/// A column of a table, named as the schema names it.
pub(crate) trait Column: Copy + AsRef<str> {
    /// The named parameter this column is bound as: `:name`.
    fn param(self) -> String {
        format!("{PARAM_MARK}{}", self.as_ref())
    }

    /// This column set to its parameter, or compared with it: `name = :name`.
    fn equals_param(self) -> String {
        format!("{}{EQUALS}{}", self.as_ref(), self.param())
    }
}

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

    /// The column names, for a `SELECT` or an `INSERT`: `a, b`.
    pub(crate) fn names(&self) -> String {
        ValueList::of_values(self.columns.iter().map(AsRef::as_ref)).written(ListForm::Commas)
    }

    /// The parameters, for an `INSERT`'s `VALUES`: `:a, :b`.
    pub(crate) fn params(&self) -> String {
        ValueList::of_values(self.columns.iter().copied().map(Column::param))
            .written(ListForm::Commas)
    }

    /// Each column set to its parameter, for an `UPDATE`: `a = :a, b = :b`.
    pub(crate) fn assignments(&self) -> String {
        ValueList::of_values(self.columns.iter().copied().map(Column::equals_param))
            .written(ListForm::Commas)
    }

    /// Each column equal to its parameter, for a `WHERE`: `a = :a AND b = :b`.
    pub(crate) fn conditions(&self) -> String {
        self.columns
            .iter()
            .copied()
            .map(Column::equals_param)
            .collect::<Vec<_>>()
            .join(AND)
    }
}

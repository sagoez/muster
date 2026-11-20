//! The ways a statement writes one of its columns.

use strum::EnumProperty;

/// The property holding what marks a named parameter.
const PARAM: &str = "param";
/// The property holding what compares a column with its parameter.
const COMPARE: &str = "compare";

/// How a statement writes a column `a`: by name, as its named parameter,
/// or equal to that parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumProperty)]
pub(crate) enum ColumnForm {
    /// `a`, in a `SELECT` list or an `INSERT`'s columns.
    Name,
    /// `:a`, in an `INSERT`'s values or where a value is bound.
    #[strum(props(param = ":"))]
    Param,
    /// `a = :a`, in an `UPDATE`'s `SET` or a `WHERE`.
    #[strum(props(param = ":", compare = " = "))]
    Equals,
}

impl ColumnForm {
    /// `column` written in this form.
    pub(crate) fn of(self, column: &str) -> String {
        let named = match self.get_str(PARAM) {
            Some(mark) => format!("{mark}{column}"),
            None => column.to_string(),
        };
        match self.get_str(COMPARE) {
            Some(compare) => format!("{column}{compare}{named}"),
            None => named,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each form writes the column as a statement needs it.
    #[test]
    fn each_form_writes_the_column() {
        assert_eq!(ColumnForm::Name.of("icon"), "icon");
        assert_eq!(ColumnForm::Param.of("icon"), ":icon");
        assert_eq!(ColumnForm::Equals.of("icon"), "icon = :icon");
    }
}

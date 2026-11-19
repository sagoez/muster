//! The ways a statement joins the columns it lists.

use strum::EnumProperty;

/// The property holding what goes between two entries.
const BETWEEN: &str = "between";

/// How a statement joins its columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumProperty)]
pub(crate) enum SqlList {
    /// `a, b`: a column list, `VALUES` or a `SET`.
    #[strum(props(between = ", "))]
    Commas,
    /// `a = :a AND b = :b`: the conditions of a `WHERE`.
    #[strum(props(between = " AND "))]
    And,
}

impl SqlList {
    /// `entries` joined this way.
    pub(crate) fn join(self, entries: &[String]) -> String {
        entries.join(self.get_str(BETWEEN).unwrap_or_default())
    }
}

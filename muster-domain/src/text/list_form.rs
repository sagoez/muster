//! The ways a list of values is written out in a sentence.

use strum::EnumProperty;

/// The property holding what comes before the first value.
const LEAD: &str = "lead";
/// The property holding what goes between values.
const BETWEEN: &str = "between";
/// The property holding what goes before the last value, when it differs
/// from what goes between the others.
const LAST: &str = "last";

/// How a list's values are joined: what goes before the first, between
/// the others, and before the last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumProperty)]
pub enum ListForm {
    /// `a, b, c`
    #[strum(props(between = ", "))]
    Commas,
    /// `a,b,c`
    #[strum(props(between = ","))]
    Tight,
    /// `a b c`
    #[strum(props(between = " "))]
    Spaces,
    /// `a|b|c`
    #[strum(props(between = "|"))]
    Pipes,
    /// `a, b or c`
    #[strum(props(between = ", ", last = " or "))]
    Either,
    /// `a, b and c`
    #[strum(props(between = ", ", last = " and "))]
    All,
    /// `exactly one of a, b or c`
    #[strum(props(lead = "exactly one of ", between = ", ", last = " or "))]
    ExactlyOne,
}

impl ListForm {
    /// What goes between two values of this form, before the last.
    pub fn between(self) -> &'static str {
        self.get_str(BETWEEN).unwrap_or_default()
    }

    /// What goes before the last value of this form.
    fn last(self) -> &'static str {
        self.get_str(LAST).unwrap_or_else(|| self.between())
    }

    /// `values` joined in this form; empty when there are none.
    pub(crate) fn join(self, values: &[String]) -> String {
        let lead = self.get_str(LEAD).unwrap_or_default();
        match values.split_last() {
            Some((last, [])) => format!("{lead}{last}"),
            Some((last, rest)) => {
                format!("{lead}{}{}{last}", rest.join(self.between()), self.last())
            },
            None => String::new(),
        }
    }
}

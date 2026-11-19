//! How a value is set apart from the sentence around it.

use std::fmt::Display;

use strum::Display;

/// What goes on both sides of a value a sentence names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display)]
pub enum Mark {
    /// Code: `` `a` ``.
    #[strum(serialize = "`")]
    Code,
    /// A literal character: `'a'`.
    #[strum(serialize = "'")]
    Quote,
}

impl Mark {
    /// `value` with this mark on both sides.
    pub fn around(self, value: impl Display) -> String {
        format!("{self}{value}{self}")
    }
}

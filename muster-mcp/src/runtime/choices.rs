//! The values an enum takes, written out the ways a description or a refusal
//! lists them, so the list is read from the enum and never typed by hand.

use std::fmt::Display;

use getset::Getters;
use strum::IntoEnumIterator;
use typed_builder::TypedBuilder;

/// Separates the values of a list.
const COMMA: &str = ", ";
/// Separates the values of a list written as alternatives.
const PIPE: &str = "|";
/// Comes before the last value of a list of alternatives.
const OR: &str = " or ";
/// Marks a value as code.
const BACKTICK: char = '`';
/// How an empty list is written.
const NONE: &str = "none";
/// Comes before the values of which exactly one is taken.
const EXACTLY_ONE_OF: &str = "exactly one of ";

/// Values to list, such as those an enum takes, in the order given.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct Choices {
    /// The values, as they are written.
    values: Vec<String>,
}

impl Choices {
    /// Every value of `E`, as it displays, in the order `E` declares them.
    pub(crate) fn of<E: IntoEnumIterator + Display>() -> Self {
        Self::of_values(E::iter())
    }

    /// `values`, as they display.
    pub(crate) fn of_values<T: Display>(values: impl IntoIterator<Item = T>) -> Self {
        Self::builder()
            .values(values.into_iter().map(|value| value.to_string()).collect())
            .build()
    }

    /// Every value of `E` marked as code, each followed by its `gloss` in
    /// parentheses: `` `a` (A) ``.
    pub(crate) fn glossed<E: IntoEnumIterator + Display + Copy>(
        gloss: fn(E) -> &'static str,
    ) -> Self {
        Self::of_values(
            E::iter().map(|value| format!("{BACKTICK}{value}{BACKTICK} ({})", gloss(value))),
        )
    }

    /// The same values, each marked as code: `` `a` ``.
    #[must_use]
    pub(crate) fn coded(self) -> Self {
        Self::of_values(
            self.values
                .into_iter()
                .map(|value| format!("{BACKTICK}{value}{BACKTICK}")),
        )
    }

    /// The values separated by commas: `a, b, c`.
    pub(crate) fn listed(&self) -> String {
        self.values().join(COMMA)
    }

    /// The values separated by commas, or `none` when there are none.
    pub(crate) fn listed_or_none(&self) -> String {
        if self.values().is_empty() {
            NONE.to_string()
        } else {
            self.listed()
        }
    }

    /// The values as alternatives: `a|b|c`.
    pub(crate) fn piped(&self) -> String {
        self.values().join(PIPE)
    }

    /// The values as the one an agent must pick: `exactly one of a, b or c`.
    pub(crate) fn exactly_one(&self) -> String {
        format!("{EXACTLY_ONE_OF}{}", self.either())
    }

    /// The values as a sentence of alternatives: `a, b or c`.
    pub(crate) fn either(&self) -> String {
        match self.values().split_last() {
            Some((last, [])) => last.clone(),
            Some((last, rest)) => format!("{}{OR}{last}", rest.join(COMMA)),
            None => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use strum::{Display, EnumIter, EnumMessage};

    use super::*;

    /// A three-valued enum to list, each value with a gloss.
    #[derive(Clone, Copy, Display, EnumIter, EnumMessage)]
    #[strum(serialize_all = "kebab-case")]
    enum Speed {
        /// The first value.
        #[strum(message = "S")]
        Slow,
        /// The second value.
        #[strum(message = "O")]
        OnTime,
        /// The last value.
        #[strum(message = "F")]
        Fast,
    }

    impl Speed {
        /// What the speed is, as a gloss.
        fn gloss(self) -> &'static str {
            self.get_message().unwrap_or_default()
        }
    }

    /// One value to list.
    #[derive(Display, EnumIter)]
    #[strum(serialize_all = "kebab-case")]
    enum Only {
        /// The only value.
        One,
    }

    /// An enum's values are listed each way, in the order it declares them.
    #[test]
    fn an_enum_is_listed_each_way_in_its_order() {
        let speeds = Choices::of::<Speed>();
        assert_eq!(speeds.listed(), "slow, on-time, fast");
        assert_eq!(speeds.piped(), "slow|on-time|fast");
        assert_eq!(speeds.either(), "slow, on-time or fast");
        assert_eq!(speeds.exactly_one(), "exactly one of slow, on-time or fast");
        assert_eq!(speeds.coded().either(), "`slow`, `on-time` or `fast`");
        assert_eq!(
            Choices::glossed(Speed::gloss).either(),
            "`slow` (S), `on-time` (O) or `fast` (F)"
        );
        assert_eq!(Choices::of::<Only>().either(), "one");
        assert_eq!(Choices::of_values([Speed::Fast]).listed_or_none(), "fast");
        assert_eq!(
            Choices::of_values(Vec::<Speed>::new()).listed_or_none(),
            "none"
        );
    }
}

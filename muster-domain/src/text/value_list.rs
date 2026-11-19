//! Values a sentence lists, in the order given: an enum's, or any that
//! display.

use std::fmt::Display;

use getset::Getters;
use strum::IntoEnumIterator;
use typed_builder::TypedBuilder;

use super::{list_form::ListForm, mark::Mark};

/// How an empty list is written where one must be.
const NONE: &str = "none";
/// Opens and closes a value's gloss.
const GLOSS_OPEN: &str = " (";
const GLOSS_CLOSE: &str = ")";

/// Values to list, in the order given.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ValueList {
    /// The values, as they are written.
    values: Vec<String>,
}

impl ValueList {
    /// Every value of `E`, as it displays, in the order `E` declares them.
    pub fn of<E: IntoEnumIterator + Display>() -> Self {
        Self::of_values(E::iter())
    }

    /// `values`, as they display.
    pub fn of_values<T: Display>(values: impl IntoIterator<Item = T>) -> Self {
        Self::builder()
            .values(values.into_iter().map(|value| value.to_string()).collect())
            .build()
    }

    /// Every value of `E` marked as code, each followed by its `gloss` in
    /// parentheses: `` `a` (A) ``.
    pub fn glossed<E: IntoEnumIterator + Display + Copy>(gloss: fn(E) -> &'static str) -> Self {
        Self::of_values(E::iter().map(|value| {
            format!(
                "{}{GLOSS_OPEN}{}{GLOSS_CLOSE}",
                Mark::Code.around(value),
                gloss(value)
            )
        }))
    }

    /// The same values, each with `mark` on both sides.
    #[must_use]
    pub fn marked(self, mark: Mark) -> Self {
        Self::of_values(self.values.into_iter().map(|value| mark.around(value)))
    }

    /// The values written in `form`; empty when there are none.
    pub fn written(&self, form: ListForm) -> String {
        form.join(self.values())
    }

    /// The values written in `form`, or `none` when there are none.
    pub fn written_or_none(&self, form: ListForm) -> String {
        if self.values().is_empty() {
            NONE.to_string()
        } else {
            self.written(form)
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

    /// An enum's values are written in each form, in the order it declares
    /// them, marked or glossed when asked.
    #[test]
    fn an_enum_is_written_in_each_form_in_its_order() {
        let speeds = ValueList::of::<Speed>();
        assert_eq!(speeds.written(ListForm::Commas), "slow, on-time, fast");
        assert_eq!(speeds.written(ListForm::Tight), "slow,on-time,fast");
        assert_eq!(speeds.written(ListForm::Spaces), "slow on-time fast");
        assert_eq!(speeds.written(ListForm::Pipes), "slow|on-time|fast");
        assert_eq!(speeds.written(ListForm::Either), "slow, on-time or fast");
        assert_eq!(speeds.written(ListForm::All), "slow, on-time and fast");
        assert_eq!(
            speeds.written(ListForm::ExactlyOne),
            "exactly one of slow, on-time or fast"
        );
        assert_eq!(
            speeds.clone().marked(Mark::Code).written(ListForm::Either),
            "`slow`, `on-time` or `fast`"
        );
        assert_eq!(
            speeds.marked(Mark::Quote).written(ListForm::All),
            "'slow', 'on-time' and 'fast'"
        );
        assert_eq!(
            ValueList::glossed(Speed::gloss).written(ListForm::Either),
            "`slow` (S), `on-time` (O) or `fast` (F)"
        );
    }

    /// One value is written alone, after any lead, and no values as nothing
    /// or as `none`.
    #[test]
    fn one_value_stands_alone_and_none_reads_none() {
        let one = ValueList::of_values([Speed::Fast]);
        assert_eq!(one.written(ListForm::Either), "fast");
        assert_eq!(one.written(ListForm::ExactlyOne), "exactly one of fast");
        assert_eq!(one.written_or_none(ListForm::Commas), "fast");
        let empty = ValueList::of_values(Vec::<Speed>::new());
        assert_eq!(empty.written(ListForm::Commas), "");
        assert_eq!(empty.written_or_none(ListForm::Commas), "none");
    }
}

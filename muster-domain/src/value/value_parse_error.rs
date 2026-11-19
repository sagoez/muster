use std::{fmt::Display, str::FromStr};

use getset::Getters;
use strum::IntoEnumIterator;
use thiserror::Error;
use typed_builder::TypedBuilder;

use crate::text::{ListForm, ValueList};

/// Text that does not read as the value it was meant to be.
#[derive(Debug, Clone, PartialEq, Eq, Error, Getters, TypedBuilder)]
#[error("expected {expected}, got '{found}'")]
#[getset(get = "pub")]
pub struct ValueParseError {
    /// The accepted form, such as `ROWSxCOLS`.
    #[builder(setter(into))]
    expected: String,
    /// The text that was refused.
    #[builder(setter(into))]
    found: String,
}

/// Reads `raw`, trimmed, as `T`, describing a refusal with `expected`.
///
/// # Errors
/// Returns [`ValueParseError`] when `T` refuses `raw`.
pub fn parse_as<T: FromStr>(raw: &str, expected: &str) -> Result<T, ValueParseError> {
    raw.trim().parse().map_err(|_| {
        ValueParseError::builder()
            .expected(expected)
            .found(raw)
            .build()
    })
}

/// Reads `raw`, trimmed, as one of `T`'s kebab-case names, listing them on a
/// refusal.
///
/// # Errors
/// Returns [`ValueParseError`] when `raw` names no variant.
pub fn parse_choice<T: FromStr + IntoEnumIterator + Display>(
    raw: &str,
) -> Result<T, ValueParseError> {
    parse_as(raw, &ValueList::of::<T>().written(ListForm::Commas))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::RestartPolicy;

    /// A choice reads its kebab-case names and lists them on a refusal.
    #[test]
    fn a_choice_lists_the_accepted_names() {
        assert_eq!(
            parse_choice::<RestartPolicy>(" on-failure ").unwrap(),
            RestartPolicy::OnFailure
        );
        assert_eq!(
            parse_choice::<RestartPolicy>("sometimes")
                .unwrap_err()
                .to_string(),
            "expected never, on-failure, always, got 'sometimes'"
        );
    }
}

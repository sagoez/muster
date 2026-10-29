use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::domain::coordination::KeyValue;

/// Most characters of a value a listing shows; reading the entry by key
/// returns it whole.
pub const LISTED_VALUE_CHARS: usize = 200;

/// A key-value entry as a listing shows it: its value cut to
/// [`LISTED_VALUE_CHARS`], marked `truncated` when cut.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ListedValue {
    /// The entry, its value cut.
    #[serde(flatten, with = "crate::adapter::wire::form")]
    #[getset(get = "pub")]
    entry: KeyValue,
    /// Whether the value was cut.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[getset(get_copy = "pub")]
    truncated: bool,
}

impl From<&KeyValue> for ListedValue {
    fn from(entry: &KeyValue) -> Self {
        let truncated = entry.value().chars().count() > LISTED_VALUE_CHARS;
        Self {
            entry: KeyValue::builder()
                .key(entry.key().clone())
                .value(entry.value().chars().take(LISTED_VALUE_CHARS).collect())
                .author(entry.author().clone())
                .updated_at(*entry.updated_at())
                .build(),
            truncated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        coordination::{Author, KvKey},
        value::EpochSeconds,
    };

    /// A long value is cut and marked; a short one is listed whole, unmarked.
    #[test]
    fn a_listed_value_is_cut_and_marked() {
        let entry = |value: String| {
            KeyValue::builder()
                .key(KvKey::try_new("blob").unwrap())
                .value(value)
                .author(Author::human())
                .updated_at(EpochSeconds::new(1))
                .build()
        };

        let long = ListedValue::from(&entry("x".repeat(LISTED_VALUE_CHARS + 1)));
        assert!(long.truncated());
        assert_eq!(long.entry().value().chars().count(), LISTED_VALUE_CHARS);
        let json = serde_json::to_value(&long).unwrap();
        assert_eq!(json["key"], "blob");
        assert_eq!(json["truncated"], true);

        let short = serde_json::to_value(ListedValue::from(&entry("4000".to_string()))).unwrap();
        assert_eq!(short["value"], "4000");
        assert!(short.get("truncated").is_none());
    }
}

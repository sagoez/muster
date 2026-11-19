use std::fmt::Display;

use getset::{CopyGetters, Getters};
use muster_client::ListedValue;
use serde::Serialize;
use typed_builder::TypedBuilder;

/// A page of the key-value entries, as `kv_read` without a key answers it.
#[derive(Debug, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ValuePageView {
    /// The entries on this page, each value cut to its listed length.
    #[getset(get = "pub")]
    entries: Vec<ListedValue>,
    /// Whether more entries follow.
    #[getset(get_copy = "pub")]
    more: bool,
    /// Where the next page starts, when more follow.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get = "pub")]
    cursor: Option<String>,
}

impl ValuePageView {
    /// `entries`, followed by more from `next` when there are any.
    pub fn of(entries: Vec<ListedValue>, next: Option<impl Display>) -> Self {
        Self::builder()
            .entries(entries)
            .more(next.is_some())
            .cursor(next.map(|cursor| cursor.to_string()))
            .build()
    }
}

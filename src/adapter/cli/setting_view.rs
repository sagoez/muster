use getset::{CopyGetters, Getters};
use serde::{Serialize, Serializer};
use typed_builder::TypedBuilder;

use crate::domain::{config::SettingKey, settings::Settings};

/// One setting as `config get --json` and `config set --json` show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct SettingView {
    /// The setting, by its kebab-case name.
    #[serde(serialize_with = "serialize_key")]
    #[getset(get_copy = "pub")]
    key: SettingKey,
    /// Its value, in the form `config set` reads.
    #[getset(get = "pub")]
    value: String,
}

impl SettingView {
    /// The setting `key` of `settings`.
    pub fn of(settings: &Settings, key: SettingKey) -> Self {
        Self {
            key,
            value: settings.value_text(key),
        }
    }
}

/// Writes `key` by its kebab-case name.
///
/// # Errors
/// Returns the serializer's error.
fn serialize_key<S: Serializer>(key: &SettingKey, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(key)
}

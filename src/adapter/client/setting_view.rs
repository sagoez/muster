use getset::{CopyGetters, Getters};
use serde::{Serialize, Serializer};
use strum::IntoEnumIterator;
use typed_builder::TypedBuilder;

use crate::domain::{config::SettingKey, settings::Settings};

/// One setting as `muster config get` and `config_get` show it.
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

/// Every setting, in key order, as `muster config list` and `config_get`
/// without a key show them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct SettingsView {
    /// Each setting.
    settings: Vec<SettingView>,
}

impl From<&Settings> for SettingsView {
    fn from(settings: &Settings) -> Self {
        Self {
            settings: SettingKey::iter()
                .map(|key| SettingView::of(settings, key))
                .collect(),
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

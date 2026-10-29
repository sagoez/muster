use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::domain::{config::SettingKey, settings::Settings};

/// One setting as `config_get` and `config_set` answer it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct SettingEntry {
    /// The setting's kebab-case key.
    key: String,
    /// Its value, in the form `config_set` reads back.
    value: String,
}

impl SettingEntry {
    /// The setting `key` of `settings`.
    pub fn read(settings: &Settings, key: SettingKey) -> Self {
        Self::builder()
            .key(key.to_string())
            .value(settings.value_text(key))
            .build()
    }
}

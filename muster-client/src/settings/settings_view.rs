use getset::Getters;
use muster_domain::{config::SettingKey, settings::Settings};
use serde::Serialize;
use strum::IntoEnumIterator;
use typed_builder::TypedBuilder;

use super::setting_view::SettingView;

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

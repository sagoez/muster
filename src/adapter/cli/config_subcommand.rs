use clap::Subcommand;

use super::args::{SETTING_KEYS, one_of};
use crate::domain::config::SettingKey;

/// Settings actions under `muster config`. Absent, every setting is listed.
#[derive(Subcommand)]
pub enum ConfigCommand {
    /// List every setting and its value.
    List,
    /// Print one setting's value.
    Get {
        /// The setting.
        #[arg(value_parser = one_of::<SettingKey>(&SETTING_KEYS))]
        key: SettingKey,
    },
    /// Change one setting.
    Set {
        /// The setting.
        #[arg(value_parser = one_of::<SettingKey>(&SETTING_KEYS))]
        key: SettingKey,
        /// Its new value, in the form `muster config get` prints.
        value: String,
    },
}

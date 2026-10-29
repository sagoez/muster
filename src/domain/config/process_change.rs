use std::path::PathBuf;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::{
    config::FieldEdit,
    process::{ReadyCheck, RestartPolicy, StopSignal},
    value::{CommandLine, Description, EnvName, EnvValue, PositiveDuration},
};

/// What `muster set` changes on one process. Every field left unset keeps the
/// process's current value.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ProcessChange {
    /// A new command.
    #[builder(default)]
    command: Option<CommandLine>,
    /// An edit to the working directory.
    #[builder(default)]
    working_dir: FieldEdit<PathBuf>,
    /// An edit to the description.
    #[builder(default)]
    description: FieldEdit<Description>,
    /// Variables to add or replace.
    #[builder(default)]
    set_env: Vec<(EnvName, EnvValue)>,
    /// Variables to remove, applied after `set_env`.
    #[builder(default)]
    unset_env: Vec<EnvName>,
    /// A new autostart flag.
    #[builder(default)]
    autostart: Option<bool>,
    /// A new restart policy.
    #[builder(default)]
    restart: Option<RestartPolicy>,
    /// A new graceful stop signal.
    #[builder(default)]
    stop_signal: Option<StopSignal>,
    /// A new grace period before force-kill.
    #[builder(default)]
    stop_grace: Option<PositiveDuration>,
    /// An edit to the readiness check.
    #[builder(default)]
    ready: FieldEdit<ReadyCheck>,
}

impl ProcessChange {
    /// Whether this change leaves every field as it is.
    pub fn is_empty(&self) -> bool {
        self.command.is_none()
            && self.working_dir.is_keep()
            && self.description.is_keep()
            && self.set_env.is_empty()
            && self.unset_env.is_empty()
            && self.autostart.is_none()
            && self.restart.is_none()
            && self.stop_signal.is_none()
            && self.stop_grace.is_none()
            && self.ready.is_keep()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_only_when_nothing_changes() {
        assert!(ProcessChange::builder().build().is_empty());
        assert!(
            !ProcessChange::builder()
                .description(FieldEdit::Clear)
                .build()
                .is_empty()
        );
        assert!(
            !ProcessChange::builder()
                .unset_env(vec![EnvName::try_new("PORT").unwrap()])
                .build()
                .is_empty()
        );
        assert!(
            !ProcessChange::builder()
                .ready(FieldEdit::Clear)
                .build()
                .is_empty()
        );
    }
}

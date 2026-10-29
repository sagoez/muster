use std::path::PathBuf;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::{
    config::FieldEdit,
    process::{ReadyCheck, RestartPolicy, StopSignal},
    value::{CommandLine, Description, EnvName, EnvValue, StopGrace},
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
    stop_grace: Option<StopGrace>,
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

    /// Whether a running process only takes this change when it next starts:
    /// a new command, working directory, environment, stop policy or
    /// readiness check. A description, autostart or restart policy needs no
    /// restart.
    pub fn needs_restart(&self) -> bool {
        self.command.is_some()
            || !self.working_dir.is_keep()
            || !self.set_env.is_empty()
            || !self.unset_env.is_empty()
            || self.stop_signal.is_some()
            || self.stop_grace.is_some()
            || !self.ready.is_keep()
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

    /// What a process reads at launch needs a restart; its description,
    /// autostart and restart policy do not.
    #[test]
    fn only_launch_fields_need_a_restart() {
        let unset = vec![EnvName::try_new("PORT").unwrap()];
        assert!(
            ProcessChange::builder()
                .unset_env(unset)
                .build()
                .needs_restart()
        );
        assert!(
            ProcessChange::builder()
                .working_dir(FieldEdit::Clear)
                .build()
                .needs_restart()
        );
        assert!(
            ProcessChange::builder()
                .ready(FieldEdit::Clear)
                .build()
                .needs_restart()
        );
        assert!(
            !ProcessChange::builder()
                .description(FieldEdit::Clear)
                .autostart(Some(true))
                .restart(Some(RestartPolicy::Always))
                .build()
                .needs_restart()
        );
    }
}

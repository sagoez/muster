use std::path::PathBuf;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::{
    config::{DeclaredPorts, FieldEdit, ProcessField},
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
    /// The ports it listens on, replacing the set; an empty set clears it.
    #[builder(default)]
    ports: Option<DeclaredPorts>,
}

impl ProcessChange {
    /// Whether this change leaves every field as it is.
    pub fn is_empty(&self) -> bool {
        self.fields().is_empty()
    }

    /// The fields this change edits, in definition order.
    pub fn fields(&self) -> Vec<ProcessField> {
        [
            (self.command.is_some(), ProcessField::Command),
            (!self.working_dir.is_keep(), ProcessField::WorkingDir),
            (!self.description.is_keep(), ProcessField::Description),
            (
                !self.set_env.is_empty() || !self.unset_env.is_empty(),
                ProcessField::Environment,
            ),
            (self.autostart.is_some(), ProcessField::Autostart),
            (self.restart.is_some(), ProcessField::Restart),
            (self.stop_signal.is_some(), ProcessField::StopSignal),
            (self.stop_grace.is_some(), ProcessField::StopGrace),
            (!self.ready.is_keep(), ProcessField::Ready),
            (self.ports.is_some(), ProcessField::Ports),
        ]
        .into_iter()
        .filter_map(|(edited, field)| edited.then_some(field))
        .collect()
    }

    /// Whether a running process only takes this change when it next starts:
    /// a new command, working directory, environment, stop policy or
    /// readiness check. A description, autostart, restart policy or ports
    /// need no restart.
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
    use crate::domain::value::LocalPort;

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

    /// A change names each field it edits once, environment sets and unsets
    /// as one field.
    #[test]
    fn a_change_names_the_fields_it_edits() {
        let change = ProcessChange::builder()
            .command(Some(CommandLine::try_new("npm start").unwrap()))
            .working_dir(FieldEdit::Clear)
            .set_env(vec![(
                EnvName::try_new("PORT").unwrap(),
                EnvValue::try_new("1").unwrap(),
            )])
            .unset_env(vec![EnvName::try_new("HOST").unwrap()])
            .restart(Some(RestartPolicy::Always))
            .build();

        assert_eq!(change.fields(), [
            ProcessField::Command,
            ProcessField::WorkingDir,
            ProcessField::Environment,
            ProcessField::Restart,
        ]);
        assert!(ProcessChange::builder().build().fields().is_empty());
    }

    /// Ports are checked before each launch, so changing them restarts
    /// nothing, and the change names only the ports field.
    #[test]
    fn a_ports_change_needs_no_restart_and_names_the_ports_field() {
        let change = ProcessChange::builder()
            .ports(Some(
                DeclaredPorts::of([LocalPort::try_new(3000).unwrap()]).unwrap(),
            ))
            .build();

        assert!(!change.needs_restart());
        assert!(!change.is_empty());
        assert_eq!(change.fields(), [ProcessField::Ports]);
        assert_eq!(
            ProcessChange::builder()
                .ports(Some(DeclaredPorts::default()))
                .build()
                .fields(),
            [ProcessField::Ports],
            "clearing the ports is a change too"
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

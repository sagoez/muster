use std::path::PathBuf;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::{
    config::{Environment, ProcessDefaults, ProcessSpec},
    process::{RestartPolicy, StopPolicy, StopSignal},
    value::{CommandLine, Description, PositiveDuration, ProcessName},
};

/// A process as `muster add` receives it, before the `default_*` settings fill
/// the fields it leaves out.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ProcessDraft {
    /// The name it is addressed by.
    name: ProcessName,
    /// The shell command it runs.
    command: CommandLine,
    /// Working directory: the project root when absent, relative to it otherwise.
    #[builder(default)]
    working_dir: Option<PathBuf>,
    /// Optional one-line description.
    #[builder(default)]
    description: Option<Description>,
    /// Environment exported to it.
    #[builder(default = Environment::empty())]
    environment: Environment,
    /// Whether `muster up` starts it, or the default.
    #[builder(default)]
    autostart: Option<bool>,
    /// Its restart policy, or the default.
    #[builder(default)]
    restart: Option<RestartPolicy>,
    /// Its graceful stop signal, or the default.
    #[builder(default)]
    stop_signal: Option<StopSignal>,
    /// Its grace period before force-kill, or the default.
    #[builder(default)]
    stop_grace: Option<PositiveDuration>,
}

impl ProcessDraft {
    /// The full definition, taking each field left out from `defaults`.
    pub fn complete(self, defaults: &ProcessDefaults) -> ProcessSpec {
        let stop = StopPolicy::builder()
            .signal(self.stop_signal.unwrap_or(*defaults.stop().signal()))
            .grace_period(self.stop_grace.unwrap_or(*defaults.stop().grace_period()))
            .build();
        ProcessSpec::builder()
            .name(self.name)
            .command(self.command)
            .working_dir(self.working_dir)
            .description(self.description)
            .environment(self.environment)
            .autostart(self.autostart.unwrap_or(*defaults.autostart()))
            .restart(self.restart.unwrap_or(*defaults.restart()))
            .stop(stop)
            .build()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn grace(seconds: u64) -> PositiveDuration {
        PositiveDuration::try_new(Duration::from_secs(seconds)).unwrap()
    }

    fn defaults() -> ProcessDefaults {
        ProcessDefaults::builder()
            .autostart(false)
            .restart(RestartPolicy::Never)
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Terminate)
                    .grace_period(grace(5))
                    .build(),
            )
            .build()
    }

    fn name() -> ProcessName {
        ProcessName::try_new("web").unwrap()
    }

    fn command() -> CommandLine {
        CommandLine::try_new("npm run dev").unwrap()
    }

    #[test]
    fn fields_left_out_take_the_defaults() {
        let spec = ProcessDraft::builder()
            .name(name())
            .command(command())
            .build()
            .complete(&defaults());

        assert!(!*spec.autostart());
        assert_eq!(*spec.restart(), RestartPolicy::Never);
        assert_eq!(*spec.stop(), *defaults().stop());
        assert!(spec.environment().is_empty());
        assert!(spec.working_dir().is_none());
    }

    #[test]
    fn stated_fields_override_the_defaults() {
        let spec = ProcessDraft::builder()
            .name(name())
            .command(command())
            .autostart(Some(true))
            .restart(Some(RestartPolicy::Always))
            .stop_signal(Some(StopSignal::Interrupt))
            .build()
            .complete(&defaults());

        assert!(*spec.autostart());
        assert_eq!(*spec.restart(), RestartPolicy::Always);
        assert_eq!(*spec.stop().signal(), StopSignal::Interrupt);
        assert_eq!(*spec.stop().grace_period(), grace(5));
    }
}

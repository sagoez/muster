use std::path::PathBuf;

use getset::{Getters, WithSetters};
use typed_builder::TypedBuilder;

use crate::domain::{
    config::{Environment, ProcessChange},
    process::{Process, ReadyCheck, RestartPolicy, StopPolicy},
    value::{CommandLine, Description, PaneId, ProcessName, ProjectRoot},
};

/// The stored definition of one process in a project's stack.
#[derive(Clone, Debug, PartialEq, Eq, Getters, WithSetters, TypedBuilder)]
#[set_with]
pub struct ProcessSpec {
    /// The name it is addressed by, unique in its project by NFC key.
    #[getset(get = "pub", set_with = "pub")]
    name: ProcessName,
    /// The shell command it runs.
    #[getset(get = "pub", set_with = "pub")]
    command: CommandLine,
    /// Working directory: the project root when absent, relative to it otherwise.
    #[getset(get = "pub", set_with = "pub")]
    #[builder(default)]
    working_dir: Option<PathBuf>,
    /// Optional one-line description.
    #[getset(get = "pub", set_with = "pub")]
    #[builder(default)]
    description: Option<Description>,
    /// Environment exported to it.
    #[getset(get = "pub", set_with = "pub")]
    environment: Environment,
    /// Whether `muster up` starts it.
    #[getset(get = "pub", set_with = "pub")]
    autostart: bool,
    /// What happens when its child exits.
    #[getset(get = "pub", set_with = "pub")]
    restart: RestartPolicy,
    /// How it is stopped.
    #[getset(get = "pub", set_with = "pub")]
    stop: StopPolicy,
    /// What each run waits for before it counts as ready, when anything.
    #[getset(get = "pub", set_with = "pub")]
    #[builder(default)]
    ready: Option<ReadyCheck>,
}

impl ProcessSpec {
    /// This definition with `change` applied; variables are set before they are
    /// unset, so a name in both ends up unset.
    pub fn with_change(self, change: &ProcessChange) -> Self {
        let environment = change
            .set_env()
            .iter()
            .fold(self.environment, |environment, (name, value)| {
                environment.with_var(name.clone(), value.clone())
            });
        let environment = change
            .unset_env()
            .iter()
            .fold(environment, |environment, name| {
                environment.without_var(name)
            });
        let stop = self
            .stop
            .with_signal(change.stop_signal().unwrap_or(*self.stop.signal()))
            .with_grace_period(change.stop_grace().unwrap_or(*self.stop.grace_period()));
        Self {
            name: self.name,
            command: change.command().clone().unwrap_or(self.command),
            working_dir: change.working_dir().clone().apply(self.working_dir),
            description: change.description().clone().apply(self.description),
            environment,
            autostart: change.autostart().unwrap_or(self.autostart),
            restart: change.restart().unwrap_or(self.restart),
            stop,
            ready: change.ready().clone().apply(self.ready),
        }
    }

    /// The directory the process launches in under `root`.
    pub fn launch_dir(&self, root: &ProjectRoot) -> PathBuf {
        match &self.working_dir {
            Some(dir) => root.as_ref().join(dir),
            None => root.as_ref().clone(),
        }
    }

    /// The process this definition describes, on pane `id` and `Pending`.
    pub fn to_process(&self, id: PaneId) -> Process {
        Process::builder()
            .id(id)
            .name(self.name.clone())
            .command(self.command.clone())
            .working_dir(self.working_dir.clone())
            .description(self.description.clone())
            .environment(self.environment.clone())
            .restart(self.restart)
            .stop(self.stop)
            .autostart(self.autostart)
            .ready(self.ready.clone())
            .build()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::{
        config::FieldEdit,
        process::{ProcessState, StopSignal},
        value::{EnvName, EnvValue, StopGrace},
    };

    fn name(raw: &str) -> EnvName {
        EnvName::try_new(raw).unwrap()
    }

    fn value(raw: &str) -> EnvValue {
        EnvValue::try_new(raw).unwrap()
    }

    fn grace(seconds: u64) -> StopGrace {
        StopGrace::try_new(Duration::from_secs(seconds)).unwrap()
    }

    fn spec() -> ProcessSpec {
        ProcessSpec::builder()
            .name(ProcessName::try_new("web").unwrap())
            .command(CommandLine::try_new("npm run dev").unwrap())
            .working_dir(Some(PathBuf::from("frontend")))
            .description(Some(Description::try_new("Vite").unwrap()))
            .environment(
                Environment::empty()
                    .with_var(name("PORT"), value("5173"))
                    .with_var(name("HOST"), value("localhost")),
            )
            .autostart(true)
            .restart(RestartPolicy::OnFailure)
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Terminate)
                    .grace_period(grace(5))
                    .build(),
            )
            .build()
    }

    fn root() -> ProjectRoot {
        ProjectRoot::try_new(PathBuf::from("/home/me/shop")).unwrap()
    }

    #[test]
    fn an_empty_change_keeps_everything() {
        assert_eq!(
            spec().with_change(&ProcessChange::builder().build()),
            spec()
        );
    }

    #[test]
    fn a_change_replaces_only_the_fields_it_names() {
        let change = ProcessChange::builder()
            .command(Some(CommandLine::try_new("pnpm dev").unwrap()))
            .working_dir(FieldEdit::Clear)
            .description(FieldEdit::Set(Description::try_new("Dev").unwrap()))
            .set_env(vec![
                (name("PORT"), value("8080")),
                (name("NEW"), value("1")),
            ])
            .unset_env(vec![name("HOST"), name("ABSENT")])
            .autostart(Some(false))
            .restart(Some(RestartPolicy::Always))
            .stop_grace(Some(grace(9)))
            .build();

        let changed = spec().with_change(&change);

        assert_eq!(changed.name(), spec().name());
        assert_eq!(changed.command().as_ref(), "pnpm dev");
        assert!(changed.working_dir().is_none());
        assert_eq!(changed.description().as_ref().unwrap().as_ref(), "Dev");
        assert_eq!(
            changed.environment(),
            &Environment::empty()
                .with_var(name("PORT"), value("8080"))
                .with_var(name("NEW"), value("1"))
        );
        assert!(!*changed.autostart());
        assert_eq!(*changed.restart(), RestartPolicy::Always);
        assert_eq!(*changed.stop().signal(), StopSignal::Terminate);
        assert_eq!(*changed.stop().grace_period(), grace(9));
    }

    #[test]
    fn a_variable_both_set_and_unset_ends_unset() {
        let change = ProcessChange::builder()
            .set_env(vec![(name("PORT"), value("8080"))])
            .unset_env(vec![name("PORT")])
            .build();

        let changed = spec().with_change(&change);

        assert!(!changed.environment().vars().contains_key(&name("PORT")));
    }

    #[test]
    fn launches_in_the_root_or_under_it() {
        assert_eq!(
            spec().launch_dir(&root()),
            PathBuf::from("/home/me/shop/frontend")
        );
        assert_eq!(
            spec().with_working_dir(None).launch_dir(&root()),
            PathBuf::from("/home/me/shop")
        );
        assert_eq!(
            spec()
                .with_working_dir(Some(PathBuf::from("/srv/data")))
                .launch_dir(&root()),
            PathBuf::from("/srv/data")
        );
    }

    #[test]
    fn a_process_starts_pending_on_its_pane() {
        let process = spec().to_process(PaneId::new(3));

        assert_eq!(*process.id(), PaneId::new(3));
        assert_eq!(*process.state(), ProcessState::Pending);
        assert_eq!(process.spec(), spec());
    }
}

use std::path::PathBuf;

use getset::{Getters, WithSetters};
use strum::IntoEnumIterator;
use typed_builder::TypedBuilder;

use crate::domain::{
    config::{
        Environment, ProcessChange, ProcessField, TooManyVars, UnconfinedDir, stays_under_root,
    },
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
    ///
    /// # Errors
    /// Returns [`TooManyVars`] when the changed environment would hold more
    /// variables than a process may have.
    pub fn with_change(self, change: &ProcessChange) -> Result<Self, TooManyVars> {
        let environment = self
            .environment
            .changed(change.set_env().iter().cloned(), change.unset_env())?;
        let stop = self
            .stop
            .with_signal(change.stop_signal().unwrap_or(*self.stop.signal()))
            .with_grace_period(change.stop_grace().unwrap_or(*self.stop.grace_period()));
        Ok(Self {
            name: self.name,
            command: change.command().clone().unwrap_or(self.command),
            working_dir: change.working_dir().clone().apply(self.working_dir),
            description: change.description().clone().apply(self.description),
            environment,
            autostart: change.autostart().unwrap_or(self.autostart),
            restart: change.restart().unwrap_or(self.restart),
            stop,
            ready: change.ready().clone().apply(self.ready),
        })
    }

    /// The fields `after` holds a different value in, in definition order.
    pub fn differing_fields(&self, after: &Self) -> Vec<ProcessField> {
        ProcessField::iter()
            .filter(|field| match field {
                ProcessField::Command => self.command != after.command,
                ProcessField::WorkingDir => self.working_dir != after.working_dir,
                ProcessField::Description => self.description != after.description,
                ProcessField::Environment => self.environment != after.environment,
                ProcessField::Autostart => self.autostart != after.autostart,
                ProcessField::Restart => self.restart != after.restart,
                ProcessField::StopSignal => self.stop.signal() != after.stop.signal(),
                ProcessField::StopGrace => self.stop.grace_period() != after.stop.grace_period(),
                ProcessField::Ready => self.ready != after.ready,
            })
            .collect()
    }

    /// The directory the process launches in under `root`.
    ///
    /// # Errors
    /// Returns the [`UnconfinedDir`] of a stored directory that leaves the
    /// root, so no launch ever runs outside the project by accident.
    pub fn launch_dir(&self, root: &ProjectRoot) -> Result<PathBuf, UnconfinedDir> {
        if let Some(unconfined) = self.unconfined_dir() {
            return Err(unconfined);
        }
        Ok(match &self.working_dir {
            Some(dir) => root.as_ref().join(dir),
            None => root.as_ref().clone(),
        })
    }

    /// The stored working directory when it is empty or relative and climbs
    /// out of the root; an absolute one is a folder a person chose with
    /// `--cwd` and is kept.
    pub fn unconfined_dir(&self) -> Option<UnconfinedDir> {
        let dir = self.working_dir.as_ref()?;
        let confined = !dir.as_os_str().is_empty() && (dir.is_absolute() || stays_under_root(dir));
        (!confined).then(|| {
            UnconfinedDir::builder()
                .name(self.name.clone())
                .dir(dir.clone())
                .build()
        })
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

    /// The variable name `raw`.
    fn name(raw: &str) -> EnvName {
        EnvName::try_new(raw).unwrap()
    }

    /// The variable value `raw`.
    fn value(raw: &str) -> EnvValue {
        EnvValue::try_new(raw).unwrap()
    }

    /// A stop grace of `seconds`.
    fn grace(seconds: u64) -> StopGrace {
        StopGrace::try_new(Duration::from_secs(seconds)).unwrap()
    }

    /// A definition with every field set.
    fn spec() -> ProcessSpec {
        ProcessSpec::builder()
            .name(ProcessName::try_new("web").unwrap())
            .command(CommandLine::try_new("npm run dev").unwrap())
            .working_dir(Some(PathBuf::from("frontend")))
            .description(Some(Description::try_new("Vite").unwrap()))
            .environment(
                Environment::of([
                    (name("PORT"), value("5173")),
                    (name("HOST"), value("localhost")),
                ])
                .unwrap(),
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

    /// The project every test definition belongs to.
    fn root() -> ProjectRoot {
        ProjectRoot::try_new(PathBuf::from("/home/me/shop")).unwrap()
    }

    #[test]
    fn an_empty_change_keeps_everything() {
        assert_eq!(
            spec()
                .with_change(&ProcessChange::builder().build())
                .unwrap(),
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

        let changed = spec().with_change(&change).unwrap();

        assert_eq!(changed.name(), spec().name());
        assert_eq!(changed.command().as_ref(), "pnpm dev");
        assert!(changed.working_dir().is_none());
        assert_eq!(changed.description().as_ref().unwrap().as_ref(), "Dev");
        assert_eq!(
            changed.environment(),
            &Environment::of([(name("PORT"), value("8080")), (name("NEW"), value("1"))]).unwrap()
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

        let changed = spec().with_change(&change).unwrap();

        assert!(!changed.environment().vars().contains_key(&name("PORT")));
    }

    #[test]
    fn launches_in_the_root_or_under_it() {
        assert_eq!(
            spec().launch_dir(&root()),
            Ok(PathBuf::from("/home/me/shop/frontend"))
        );
        assert_eq!(
            spec().with_working_dir(None).launch_dir(&root()),
            Ok(PathBuf::from("/home/me/shop"))
        );
        assert_eq!(
            spec()
                .with_working_dir(Some(PathBuf::from("/srv/data")))
                .launch_dir(&root()),
            Ok(PathBuf::from("/srv/data"))
        );
    }

    /// A stored relative directory that climbs out of the root, or an empty
    /// one, never launches, and the refusal names the process and the fix.
    #[test]
    fn an_unconfined_dir_never_launches() {
        for escaping in ["../../../../etc", "a/../..", ""] {
            let unconfined = spec().with_working_dir(Some(PathBuf::from(escaping)));
            let refused = unconfined.launch_dir(&root()).unwrap_err();
            assert_eq!(Some(refused.clone()), unconfined.unconfined_dir());
            assert_eq!(refused.name(), spec().name());
            assert!(
                refused.to_string().contains("`muster set 'web' --no-cwd`"),
                "{refused}"
            );
        }
        for confined in ["frontend/../src", "/srv/elsewhere"] {
            let spec = spec().with_working_dir(Some(PathBuf::from(confined)));
            assert_eq!(spec.unconfined_dir(), None, "{confined}");
        }
    }

    #[test]
    fn a_process_starts_pending_on_its_pane() {
        let process = spec().to_process(PaneId::new(3));

        assert_eq!(*process.id(), PaneId::new(3));
        assert_eq!(*process.state(), ProcessState::Pending);
        assert_eq!(process.spec(), spec());
    }
}

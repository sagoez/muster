use std::collections::HashSet;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::{
    config::{ConfigError, ImportMode, ImportPlan, ProcessSpec},
    project::Project,
    value::ProcessName,
};

/// A project and the processes defined in it, in insertion order.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Stack {
    /// The project the processes belong to.
    project: Project,
    /// Its process definitions, unique by NFC key.
    processes: Vec<ProcessSpec>,
}

impl Stack {
    /// The process whose name has the same key as `name`.
    pub fn process(&self, name: &ProcessName) -> Option<&ProcessSpec> {
        let key = name.key();
        self.processes.iter().find(|spec| spec.name().key() == key)
    }

    /// The process named `name`.
    ///
    /// # Errors
    /// Returns [`ConfigError::UnknownProcess`] listing the defined names when
    /// there is none.
    pub fn require(&self, name: &ProcessName) -> Result<&ProcessSpec, ConfigError> {
        self.process(name)
            .ok_or_else(|| ConfigError::UnknownProcess {
                name: name.clone(),
                valid: self.names(),
            })
    }

    /// Every defined name, in stack order.
    pub fn names(&self) -> Vec<ProcessName> {
        self.processes
            .iter()
            .map(|spec| spec.name().clone())
            .collect()
    }

    /// Refuses two specs whose names share a key.
    ///
    /// # Errors
    /// Returns [`ConfigError::DuplicateProcessName`] naming the second of them.
    pub fn check_unique(processes: &[ProcessSpec]) -> Result<(), ConfigError> {
        let mut seen = HashSet::new();
        match processes
            .iter()
            .find(|spec| !seen.insert(spec.name().key()))
        {
            Some(duplicate) => Err(ConfigError::DuplicateProcessName {
                name: duplicate.name().clone(),
            }),
            None => Ok(()),
        }
    }

    /// What importing `incoming` under `mode` adds, updates and removes. An
    /// incoming definition equal to the stored one is neither added nor updated.
    ///
    /// # Errors
    /// Returns [`ConfigError::DuplicateProcessName`] when `incoming` repeats a
    /// name, and [`ConfigError::ImportConflict`] listing every already-defined
    /// name under [`ImportMode::Strict`].
    pub fn plan_import(
        &self,
        incoming: &[ProcessSpec],
        mode: ImportMode,
    ) -> Result<ImportPlan, ConfigError> {
        Self::check_unique(incoming)?;
        let (existing, added): (Vec<&ProcessSpec>, Vec<&ProcessSpec>) = incoming
            .iter()
            .partition(|spec| self.process(spec.name()).is_some());
        if mode == ImportMode::Strict && !existing.is_empty() {
            return Err(ConfigError::ImportConflict {
                names: names_of(&existing),
            });
        }
        let updated: Vec<&ProcessSpec> = existing
            .into_iter()
            .filter(|spec| {
                self.process(spec.name())
                    .is_none_or(|stored| stored.differs_outside_readiness(spec))
            })
            .collect();
        let removed = match mode {
            ImportMode::Replace => {
                let kept: HashSet<String> = incoming.iter().map(|spec| spec.name().key()).collect();
                self.processes
                    .iter()
                    .filter(|spec| !kept.contains(&spec.name().key()))
                    .map(|spec| spec.name().clone())
                    .collect()
            },
            ImportMode::Strict | ImportMode::Merge => Vec::new(),
        };
        Ok(ImportPlan::builder()
            .added(names_of(&added))
            .updated(names_of(&updated))
            .removed(removed)
            .build())
    }
}

/// The names of `specs`, in order.
fn names_of(specs: &[&ProcessSpec]) -> Vec<ProcessName> {
    specs.iter().map(|spec| spec.name().clone()).collect()
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use super::*;
    use crate::domain::{
        config::Environment,
        process::{ReadyCheck, ReadyProbe, RestartPolicy, StopPolicy, StopSignal},
        value::{CommandLine, PositiveDuration, ProjectName, ProjectRoot},
    };

    /// `café` with a precomposed `é`.
    const CAFE_COMPOSED: &str = "caf\u{e9}";
    /// `café` with `e` and a combining acute accent.
    const CAFE_DECOMPOSED: &str = "cafe\u{301}";

    fn spec(name: &str, command: &str) -> ProcessSpec {
        ProcessSpec::builder()
            .name(ProcessName::try_new(name).unwrap())
            .command(CommandLine::try_new(command).unwrap())
            .environment(Environment::empty())
            .autostart(false)
            .restart(RestartPolicy::Never)
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Terminate)
                    .grace_period(PositiveDuration::try_new(Duration::from_secs(5)).unwrap())
                    .build(),
            )
            .build()
    }

    fn stack(processes: Vec<ProcessSpec>) -> Stack {
        let root = ProjectRoot::try_new(PathBuf::from("/home/me/shop")).unwrap();
        Stack::builder()
            .project(
                Project::builder()
                    .root(root)
                    .name(ProjectName::try_new("shop").unwrap())
                    .build(),
            )
            .processes(processes)
            .build()
    }

    fn names(raw: &[&str]) -> Vec<ProcessName> {
        raw.iter()
            .map(|name| ProcessName::try_new(*name).unwrap())
            .collect()
    }

    fn current() -> Stack {
        stack(vec![spec("web", "npm run dev"), spec("api", "cargo run")])
    }

    #[test]
    fn finds_a_process_by_nfc_key() {
        let stack = stack(vec![spec(CAFE_COMPOSED, "true")]);

        assert!(
            stack
                .process(&ProcessName::try_new(CAFE_DECOMPOSED).unwrap())
                .is_some()
        );
        assert!(matches!(
            stack.require(&ProcessName::try_new("tea").unwrap()),
            Err(ConfigError::UnknownProcess { valid, .. }) if valid == names(&[CAFE_COMPOSED])
        ));
    }

    #[test]
    fn check_unique_refuses_names_equal_in_normal_form_c() {
        let error =
            Stack::check_unique(&[spec(CAFE_COMPOSED, "true"), spec(CAFE_DECOMPOSED, "true")])
                .unwrap_err();

        assert!(matches!(
            error,
            ConfigError::DuplicateProcessName { name } if name.as_ref() == CAFE_DECOMPOSED
        ));
        assert!(Stack::check_unique(&[spec("web", "true"), spec("api", "true")]).is_ok());
    }

    #[test]
    fn strict_import_adds_new_names() {
        let plan = current()
            .plan_import(&[spec("worker", "bin/worker")], ImportMode::Strict)
            .unwrap();

        assert_eq!(*plan.added(), names(&["worker"]));
        assert!(plan.updated().is_empty());
        assert!(plan.removed().is_empty());
    }

    /// Strict refuses every already-defined name, even an identical definition.
    #[test]
    fn strict_import_refuses_every_defined_name() {
        let error = current()
            .plan_import(
                &[
                    spec("web", "npm run dev"),
                    spec("worker", "bin/worker"),
                    spec("api", "cargo run --release"),
                ],
                ImportMode::Strict,
            )
            .unwrap_err();

        assert!(matches!(
            error,
            ConfigError::ImportConflict { names: conflicts } if conflicts == names(&["web", "api"])
        ));
    }

    #[test]
    fn merge_import_updates_changed_names_and_keeps_the_rest() {
        let plan = current()
            .plan_import(
                &[
                    spec("web", "npm run dev"),
                    spec("api", "cargo run --release"),
                    spec("worker", "bin/worker"),
                ],
                ImportMode::Merge,
            )
            .unwrap();

        assert_eq!(*plan.added(), names(&["worker"]));
        assert_eq!(*plan.updated(), names(&["api"]));
        assert!(plan.removed().is_empty());
    }

    #[test]
    fn replace_import_removes_names_the_file_leaves_out() {
        let plan = current()
            .plan_import(
                &[
                    spec("api", "cargo run --release"),
                    spec("worker", "bin/worker"),
                ],
                ImportMode::Replace,
            )
            .unwrap();

        assert_eq!(*plan.added(), names(&["worker"]));
        assert_eq!(*plan.updated(), names(&["api"]));
        assert_eq!(*plan.removed(), names(&["web"]));
    }

    #[test]
    fn replacing_with_the_same_stack_changes_nothing() {
        let plan = current()
            .plan_import(current().processes(), ImportMode::Replace)
            .unwrap();

        assert!(plan.is_empty());
    }

    /// An import does not manage readiness checks, so a stored one never
    /// makes an otherwise identical definition count as updated.
    #[test]
    fn a_stored_readiness_check_does_not_count_as_a_change() {
        let check = ReadyCheck::builder()
            .probe(ReadyProbe::from_stored("port", "3000").unwrap())
            .timeout(PositiveDuration::try_new(Duration::from_secs(30)).unwrap())
            .build();
        let stored = stack(vec![spec("web", "npm run dev").with_ready(Some(check))]);

        let plan = stored
            .plan_import(&[spec("web", "npm run dev")], ImportMode::Merge)
            .unwrap();

        assert!(plan.is_empty());
    }

    #[test]
    fn every_mode_refuses_a_repeated_name() {
        for mode in [ImportMode::Strict, ImportMode::Merge, ImportMode::Replace] {
            assert!(matches!(
                current().plan_import(&[spec("x", "true"), spec("x", "false")], mode),
                Err(ConfigError::DuplicateProcessName { .. })
            ));
        }
    }
}

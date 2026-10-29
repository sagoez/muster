use typed_builder::TypedBuilder;

use crate::domain::{
    config::{
        ConfigError, ImportMode, ImportPlan, ProcessChange, ProcessDraft, ProcessSpec,
        Registration, SettingChange,
    },
    port::{SettingsStore, StackStore, StateChangeNotifier},
    settings::Settings,
    value::{ProcessName, ProjectRoot},
};

/// The one use case the CLI and MCP drive to edit stored state: each method
/// writes through its store and, once that write committed, tells the notifier.
#[derive(TypedBuilder)]
pub struct StackEditor<'a> {
    /// Where each project's processes are stored.
    stacks: &'a dyn StackStore,
    /// Where the settings are stored.
    settings: &'a dyn SettingsStore,
    /// Told after each successful write.
    notifier: &'a dyn StateChangeNotifier,
}

impl StackEditor<'_> {
    /// Adds `draft` to the project at `root`, filling what it leaves out from
    /// the `default_*` settings and registering `root` when it is new.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the settings cannot be read or the store
    /// refuses the process, such as a taken name.
    pub fn add(
        &self,
        root: &ProjectRoot,
        draft: ProcessDraft,
    ) -> Result<(Registration, ProcessSpec), ConfigError> {
        let spec = draft.complete(&self.settings.load()?.process_defaults());
        let registration = self.stacks.add_process(root, &spec)?;
        self.notifier.stack_changed(root);
        Ok((registration, spec))
    }

    /// Applies `change` to the process `name` and returns its new definition.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the project or process is unknown or the
    /// store cannot be written.
    pub fn change(
        &self,
        root: &ProjectRoot,
        name: &ProcessName,
        change: &ProcessChange,
    ) -> Result<ProcessSpec, ConfigError> {
        let spec = self.stacks.change_process(root, name, change)?;
        self.notifier.stack_changed(root);
        Ok(spec)
    }

    /// Removes the process `name` and returns what it was.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the project or process is unknown or the
    /// store cannot be written.
    pub fn remove(
        &self,
        root: &ProjectRoot,
        name: &ProcessName,
    ) -> Result<ProcessSpec, ConfigError> {
        let spec = self.stacks.remove_process(root, name)?;
        self.notifier.stack_changed(root);
        Ok(spec)
    }

    /// Imports `processes` into the project at `root` under `mode`, in one
    /// transaction, registering `root` when it is new.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the import conflicts, repeats a name, or
    /// the store cannot be written; nothing is written then.
    pub fn import(
        &self,
        root: &ProjectRoot,
        processes: &[ProcessSpec],
        mode: ImportMode,
    ) -> Result<(Registration, ImportPlan), ConfigError> {
        let imported = self.stacks.import(root, processes, mode)?;
        self.notifier.stack_changed(root);
        Ok(imported)
    }

    /// Applies `change` to the settings and returns the result.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the value is refused or the store cannot
    /// be written.
    pub fn configure(&self, change: &SettingChange) -> Result<Settings, ConfigError> {
        let settings = self.settings.apply(change)?;
        self.notifier.settings_changed();
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, path::PathBuf, time::Duration};

    use super::*;
    use crate::domain::{
        color_scheme::ColorScheme,
        config::{Environment, Stack},
        process::{RestartPolicy, StopSignal},
        project::Project,
        pty::PtySize,
        value::{
            Cols, CommandLine, DetachKey, ProjectName, Rows, ScrollbackLines, StackRevision,
            StopGrace,
        },
    };

    /// One step the fakes record, so a test can check what happened in order.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Step {
        /// A store committed a write.
        Wrote,
        /// The notifier heard a stack change.
        StackChanged(ProjectRoot),
        /// The notifier heard a settings change.
        SettingsChanged,
    }

    /// Fakes every port over one shared log; `fail` makes every write refuse.
    struct Fakes {
        log: RefCell<Vec<Step>>,
        added: RefCell<Vec<ProcessSpec>>,
        settings: RefCell<Settings>,
        fail: bool,
    }

    impl Fakes {
        fn new(fail: bool) -> Self {
            Self {
                log: RefCell::new(Vec::new()),
                added: RefCell::new(Vec::new()),
                settings: RefCell::new(settings()),
                fail,
            }
        }

        fn editor(&self) -> StackEditor<'_> {
            StackEditor::builder()
                .stacks(self)
                .settings(self)
                .notifier(self)
                .build()
        }

        /// Records a write, or refuses it when failing.
        fn write(&self) -> Result<(), ConfigError> {
            if self.fail {
                return Err(ConfigError::Database("refused".to_string()));
            }
            self.log.borrow_mut().push(Step::Wrote);
            Ok(())
        }

        fn steps(&self) -> Vec<Step> {
            self.log.borrow().clone()
        }
    }

    impl StackStore for Fakes {
        fn stack(&self, _root: &ProjectRoot) -> Result<Option<Stack>, ConfigError> {
            Ok(None)
        }

        fn stack_revision(
            &self,
            _root: &ProjectRoot,
        ) -> Result<Option<StackRevision>, ConfigError> {
            Ok(None)
        }

        fn add_process(
            &self,
            _root: &ProjectRoot,
            spec: &ProcessSpec,
        ) -> Result<Registration, ConfigError> {
            self.write()?;
            self.added.borrow_mut().push(spec.clone());
            Ok(registration())
        }

        fn change_process(
            &self,
            _root: &ProjectRoot,
            _name: &ProcessName,
            change: &ProcessChange,
        ) -> Result<ProcessSpec, ConfigError> {
            self.write()?;
            Ok(spec().with_change(change))
        }

        fn remove_process(
            &self,
            _root: &ProjectRoot,
            _name: &ProcessName,
        ) -> Result<ProcessSpec, ConfigError> {
            self.write()?;
            Ok(spec())
        }

        fn import(
            &self,
            _root: &ProjectRoot,
            processes: &[ProcessSpec],
            mode: ImportMode,
        ) -> Result<(Registration, ImportPlan), ConfigError> {
            self.write()?;
            let stack = Stack::builder()
                .project(registration().project().clone())
                .processes(Vec::new())
                .build();
            Ok((registration(), stack.plan_import(processes, mode)?))
        }
    }

    impl SettingsStore for Fakes {
        fn load(&self) -> Result<Settings, ConfigError> {
            Ok(self.settings.borrow().clone())
        }

        fn apply(&self, change: &SettingChange) -> Result<Settings, ConfigError> {
            self.write()?;
            let changed = self.settings.borrow().clone().with_change(change)?;
            *self.settings.borrow_mut() = changed.clone();
            Ok(changed)
        }
    }

    impl StateChangeNotifier for Fakes {
        fn stack_changed(&self, root: &ProjectRoot) {
            self.log.borrow_mut().push(Step::StackChanged(root.clone()));
        }

        fn settings_changed(&self) {
            self.log.borrow_mut().push(Step::SettingsChanged);
        }
    }

    fn root() -> ProjectRoot {
        ProjectRoot::try_new(PathBuf::from("/home/me/shop")).unwrap()
    }

    fn registration() -> Registration {
        Registration::builder()
            .project(
                Project::builder()
                    .root(root())
                    .name(ProjectName::try_new("shop").unwrap())
                    .build(),
            )
            .newly_registered(true)
            .build()
    }

    fn grace(seconds: u64) -> StopGrace {
        StopGrace::try_new(Duration::from_secs(seconds)).unwrap()
    }

    fn settings() -> Settings {
        Settings::builder()
            .desktop_notifications(true)
            .default_size(
                PtySize::builder()
                    .rows(Rows::new(24))
                    .cols(Cols::new(80))
                    .build(),
            )
            .detach_key(DetachKey::try_new("ctrl-\\").unwrap())
            .log_max_size("10MiB".parse().unwrap())
            .scrollback(ScrollbackLines::try_new(1000).unwrap())
            .color_scheme(ColorScheme::Dark)
            .default_autostart(true)
            .default_restart(RestartPolicy::OnFailure)
            .default_stop_signal(StopSignal::Interrupt)
            .default_stop_grace(grace(7))
            .build()
    }

    fn name() -> ProcessName {
        ProcessName::try_new("web").unwrap()
    }

    fn draft() -> ProcessDraft {
        ProcessDraft::builder()
            .name(name())
            .command(CommandLine::try_new("npm run dev").unwrap())
            .build()
    }

    fn spec() -> ProcessSpec {
        draft().complete(&settings().process_defaults())
    }

    /// The write commits first, then the notifier hears the project's root.
    fn assert_wrote_then_notified(fakes: &Fakes) {
        assert_eq!(fakes.steps(), [Step::Wrote, Step::StackChanged(root())]);
    }

    #[test]
    fn add_completes_the_draft_from_the_default_settings() {
        let fakes = Fakes::new(false);

        let (registration, spec) = fakes.editor().add(&root(), draft()).unwrap();

        assert!(*registration.newly_registered());
        assert!(*spec.autostart());
        assert_eq!(*spec.restart(), RestartPolicy::OnFailure);
        assert_eq!(*spec.stop().signal(), StopSignal::Interrupt);
        assert_eq!(*spec.stop().grace_period(), grace(7));
        assert_eq!(spec.environment(), &Environment::empty());
        assert_eq!(*fakes.added.borrow(), [spec]);
        assert_wrote_then_notified(&fakes);
    }

    #[test]
    fn change_returns_the_new_definition_after_its_write() {
        let fakes = Fakes::new(false);

        let changed = fakes
            .editor()
            .change(&root(), &name(), &change_of())
            .unwrap();

        assert!(!*changed.autostart());
        assert_wrote_then_notified(&fakes);
    }

    #[test]
    fn remove_notifies_after_its_write() {
        let fakes = Fakes::new(false);

        fakes.editor().remove(&root(), &name()).unwrap();

        assert_wrote_then_notified(&fakes);
    }

    #[test]
    fn import_notifies_after_its_write() {
        let fakes = Fakes::new(false);

        let (_, plan) = fakes
            .editor()
            .import(&root(), &[spec()], ImportMode::Strict)
            .unwrap();

        assert_eq!(*plan.added(), [name()]);
        assert_wrote_then_notified(&fakes);
    }

    /// The change the `change` edits apply.
    fn change_of() -> ProcessChange {
        ProcessChange::builder().autostart(Some(false)).build()
    }

    #[test]
    fn configure_notifies_a_settings_change_after_the_write() {
        let fakes = Fakes::new(false);

        let settings = fakes
            .editor()
            .configure(&SettingChange::Scrollback(
                ScrollbackLines::try_new(50).unwrap(),
            ))
            .unwrap();

        assert_eq!(settings.scrollback().into_inner(), 50);
        assert_eq!(fakes.steps(), [Step::Wrote, Step::SettingsChanged]);
    }

    /// A refused write is never announced.
    #[test]
    fn a_failed_write_notifies_nothing() {
        let fakes = Fakes::new(true);
        let editor = fakes.editor();

        assert!(editor.add(&root(), draft()).is_err());
        assert!(editor.change(&root(), &name(), &change_of()).is_err());
        assert!(editor.remove(&root(), &name()).is_err());
        assert!(
            editor
                .import(&root(), &[spec()], ImportMode::Merge)
                .is_err()
        );
        assert!(
            editor
                .configure(&SettingChange::DesktopNotifications(false))
                .is_err()
        );
        assert!(fakes.steps().is_empty());
    }

    /// A value the domain refuses is refused before any notification.
    #[test]
    fn an_invalid_setting_notifies_nothing() {
        let fakes = Fakes::new(false);
        let tiny = PtySize::builder()
            .rows(Rows::new(0))
            .cols(Cols::new(0))
            .build();

        let error = fakes
            .editor()
            .configure(&SettingChange::DefaultSize(tiny))
            .unwrap_err();

        assert!(matches!(error, ConfigError::InvalidDefaultSize { .. }));
        assert!(!fakes.steps().contains(&Step::SettingsChanged));
    }
}

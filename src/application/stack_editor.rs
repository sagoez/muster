use typed_builder::TypedBuilder;

use crate::domain::{
    config::{
        AppliedChange, ConfigError, ImportMode, ImportPlan, ProcessChange, ProcessDraft,
        ProcessSpec, Registration, SettingChange,
    },
    coordination::Author,
    port::{SettingsStore, StackStore, StateChangeNotifier},
    project::Project,
    settings::Settings,
    value::{IdleStop, ProcessName, ProjectRoot},
};

/// The one use case the CLI and MCP drive to edit stored state: each method
/// writes through its store and, once that write committed, tells the notifier.
/// The store keeps an agent's edit in the project's edit log in that same
/// write.
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
    /// Adds `draft` for `author` to the project at `root`, filling what it
    /// leaves out from the `default_*` settings and registering `root` when
    /// it is new.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the settings cannot be read or the store
    /// refuses the process, such as a taken name.
    pub fn add(
        &self,
        root: &ProjectRoot,
        draft: ProcessDraft,
        author: &Author,
    ) -> Result<(Registration, ProcessSpec), ConfigError> {
        let spec = draft.complete(&self.settings.load()?.process_defaults());
        let registration = self.stacks.add_process(root, &spec, author)?;
        self.told(root, &registration);
        Ok((registration, spec))
    }

    /// Applies `change` for `author` to the process `name` and returns its
    /// new definition and whether it changed; a change to the stored values
    /// tells the notifier nothing.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the project or process is unknown or the
    /// store cannot be written.
    pub fn change(
        &self,
        root: &ProjectRoot,
        name: &ProcessName,
        change: &ProcessChange,
        author: &Author,
    ) -> Result<AppliedChange, ConfigError> {
        let applied = self.stacks.change_process(root, name, change, author)?;
        if applied.changed() {
            self.notifier.stack_changed(root);
        }
        Ok(applied)
    }

    /// Removes the process `name` for `author` and returns what it was.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the project or process is unknown or the
    /// store cannot be written.
    pub fn remove(
        &self,
        root: &ProjectRoot,
        name: &ProcessName,
        author: &Author,
    ) -> Result<ProcessSpec, ConfigError> {
        let spec = self.stacks.remove_process(root, name, author)?;
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
        let (registration, plan) = self.stacks.import(root, processes, mode)?;
        self.told(root, &registration);
        Ok((registration, plan))
    }

    /// Tells the notifier of a write to the project at `root` that
    /// `registration` describes: its registration first when it was new,
    /// then the change to its stack.
    fn told(&self, root: &ProjectRoot, registration: &Registration) {
        if *registration.newly_registered() {
            self.notifier.project_registered(root);
        }
        self.notifier.stack_changed(root);
    }

    /// Sets the project's own idle stop, or clears it (`None`) to follow the
    /// setting, and returns the project as stored.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the project is unknown or the store
    /// cannot be written.
    pub fn set_idle_stop(
        &self,
        root: &ProjectRoot,
        idle_stop: Option<IdleStop>,
    ) -> Result<Project, ConfigError> {
        let project = self.stacks.set_idle_stop(root, idle_stop)?;
        self.notifier.stack_changed(root);
        Ok(project)
    }

    /// Applies `change` for `author` to the settings and returns the result,
    /// telling the notifier only when the stored settings changed.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the value is refused or the store cannot
    /// be written.
    pub fn configure(
        &self,
        change: &SettingChange,
        author: &Author,
    ) -> Result<Settings, ConfigError> {
        let applied = self.settings.apply(change, author)?;
        if applied.changed() {
            self.notifier.settings_changed();
        }
        Ok(applied.settings().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::{
            editor_fakes::EditorFakes,
            editor_fixtures::{draft, grace, name, root, spec},
            editor_step::EditorStep,
        },
        *,
    };
    use crate::domain::{
        config::Environment,
        process::{RestartPolicy, StopSignal},
        value::ScrollbackLines,
    };

    /// The write commits first, then the notifier hears the project's root.
    fn assert_wrote_then_notified(fakes: &EditorFakes, write: EditorStep) {
        assert_eq!(fakes.steps(), [write, EditorStep::StackChanged(root())]);
    }

    /// The write commits first, then the notifier hears the project was
    /// registered, then that its stack changed.
    fn assert_registered_then_notified(fakes: &EditorFakes, write: EditorStep) {
        assert_eq!(fakes.steps(), [
            write,
            EditorStep::ProjectRegistered(root()),
            EditorStep::StackChanged(root())
        ]);
    }

    /// The agent every attributed edit is made by.
    fn agent() -> Author {
        Author::agent("claude")
    }

    #[test]
    fn add_completes_the_draft_from_the_default_settings() {
        let fakes = EditorFakes::new(false);

        let (registration, spec) = fakes
            .editor()
            .add(&root(), draft(), &Author::human())
            .unwrap();

        assert!(*registration.newly_registered());
        assert!(*spec.autostart());
        assert_eq!(*spec.restart(), RestartPolicy::OnFailure);
        assert_eq!(*spec.stop().signal(), StopSignal::Interrupt);
        assert_eq!(*spec.stop().grace_period(), grace(7));
        assert_eq!(spec.environment(), &Environment::empty());
        assert_eq!(fakes.added(), [spec]);
        assert_registered_then_notified(&fakes, EditorStep::WroteFor(Author::human()));
    }

    /// Only a write that registered its project tells the notifier so; one
    /// to a project registered already tells it only of the stack change.
    #[test]
    fn only_a_new_project_is_told_as_registered() {
        let known = EditorFakes::new(false).over_known_project();

        known
            .editor()
            .add(&root(), draft(), &Author::human())
            .unwrap();
        known
            .editor()
            .import(&root(), &[spec()], ImportMode::Strict)
            .unwrap();

        assert_eq!(known.steps(), [
            EditorStep::WroteFor(Author::human()),
            EditorStep::StackChanged(root()),
            EditorStep::Wrote,
            EditorStep::StackChanged(root()),
        ]);
    }

    /// The agent making a change reaches the store's write, which records it.
    #[test]
    fn change_returns_the_new_definition_after_its_write() {
        let fakes = EditorFakes::new(false);

        let changed = fakes
            .editor()
            .change(&root(), &name(), &change_of(), &agent())
            .unwrap();

        assert!(changed.changed());
        assert!(!*changed.spec().autostart());
        assert_wrote_then_notified(&fakes, EditorStep::WroteFor(agent()));
    }

    /// A change to the values already stored succeeds saying it changed
    /// nothing, and is never announced.
    #[test]
    fn a_change_to_the_stored_values_notifies_nothing() {
        let fakes = EditorFakes::new(false);
        let same = ProcessChange::builder()
            .autostart(Some(*spec().autostart()))
            .build();

        let applied = fakes
            .editor()
            .change(&root(), &name(), &same, &agent())
            .unwrap();

        assert!(!applied.changed());
        assert_eq!(applied.spec(), &spec());
        assert_eq!(fakes.steps(), [EditorStep::WroteFor(agent())]);
    }

    #[test]
    fn remove_notifies_after_its_write() {
        let fakes = EditorFakes::new(false);

        fakes.editor().remove(&root(), &name(), &agent()).unwrap();

        assert_wrote_then_notified(&fakes, EditorStep::WroteFor(agent()));
    }

    #[test]
    fn import_notifies_after_its_write() {
        let fakes = EditorFakes::new(false);

        let (_, plan) = fakes
            .editor()
            .import(&root(), &[spec()], ImportMode::Strict)
            .unwrap();

        assert_eq!(*plan.added(), [name()]);
        assert_registered_then_notified(&fakes, EditorStep::Wrote);
    }

    /// The change the `change` edits apply.
    fn change_of() -> ProcessChange {
        ProcessChange::builder().autostart(Some(false)).build()
    }

    #[test]
    fn configure_notifies_a_settings_change_after_the_write() {
        let fakes = EditorFakes::new(false);

        let settings = fakes
            .editor()
            .configure(
                &SettingChange::Scrollback(ScrollbackLines::try_new(50).unwrap()),
                &agent(),
            )
            .unwrap();

        assert_eq!(settings.scrollback().into_inner(), 50);
        assert_eq!(fakes.steps(), [
            EditorStep::Wrote,
            EditorStep::SettingsChanged
        ]);
    }

    /// A change to the value already stored is never announced.
    #[test]
    fn configure_to_the_stored_value_notifies_nothing() {
        let fakes = EditorFakes::new(false);
        let change = SettingChange::Scrollback(ScrollbackLines::try_new(50).unwrap());
        fakes.editor().configure(&change, &agent()).unwrap();

        fakes.editor().configure(&change, &agent()).unwrap();

        assert_eq!(fakes.steps(), [
            EditorStep::Wrote,
            EditorStep::SettingsChanged,
            EditorStep::Wrote
        ]);
    }

    #[test]
    fn set_idle_stop_notifies_after_its_write_and_not_on_failure() {
        let fakes = EditorFakes::new(false);
        let idle_stop: IdleStop = "5m".parse().unwrap();

        let project = fakes
            .editor()
            .set_idle_stop(&root(), Some(idle_stop))
            .unwrap();

        assert_eq!(*project.idle_stop(), Some(idle_stop));
        assert_wrote_then_notified(&fakes, EditorStep::Wrote);
        let failing = EditorFakes::new(true);
        assert!(failing.editor().set_idle_stop(&root(), None).is_err());
        assert!(failing.steps().is_empty());
    }

    /// A refused write is never announced.
    #[test]
    fn a_failed_write_notifies_nothing() {
        let fakes = EditorFakes::new(true);
        let editor = fakes.editor();

        assert!(editor.add(&root(), draft(), &agent()).is_err());
        assert!(
            editor
                .change(&root(), &name(), &change_of(), &agent())
                .is_err()
        );
        assert!(editor.remove(&root(), &name(), &agent()).is_err());
        assert!(
            editor
                .import(&root(), &[spec()], ImportMode::Merge)
                .is_err()
        );
        assert!(
            editor
                .configure(&SettingChange::DesktopNotifications(false), &agent())
                .is_err()
        );
        assert!(fakes.steps().is_empty());
    }
}

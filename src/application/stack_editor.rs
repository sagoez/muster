use typed_builder::TypedBuilder;

use crate::domain::{
    config::{
        ConfigError, ImportMode, ImportPlan, ProcessChange, ProcessDraft, ProcessSpec,
        Registration, SettingChange,
    },
    coordination::Author,
    port::{SettingsStore, StackStore, StateChangeNotifier},
    settings::Settings,
    value::{ProcessName, ProjectRoot},
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
        self.notifier.stack_changed(root);
        Ok((registration, spec))
    }

    /// Applies `change` for `author` to the process `name` and returns its
    /// new definition.
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
    ) -> Result<ProcessSpec, ConfigError> {
        let spec = self.stacks.change_process(root, name, change, author)?;
        self.notifier.stack_changed(root);
        Ok(spec)
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
        let imported = self.stacks.import(root, processes, mode)?;
        self.notifier.stack_changed(root);
        Ok(imported)
    }

    /// Applies `change` for `author` to the settings and returns the result.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the value is refused or the store cannot
    /// be written.
    pub fn configure(
        &self,
        change: &SettingChange,
        author: &Author,
    ) -> Result<Settings, ConfigError> {
        let settings = self.settings.apply(change, author)?;
        self.notifier.settings_changed();
        Ok(settings)
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
        assert_wrote_then_notified(&fakes, EditorStep::WroteFor(Author::human()));
    }

    /// The agent making a change reaches the store's write, which records it.
    #[test]
    fn change_returns_the_new_definition_after_its_write() {
        let fakes = EditorFakes::new(false);

        let changed = fakes
            .editor()
            .change(&root(), &name(), &change_of(), &agent())
            .unwrap();

        assert!(!*changed.autostart());
        assert_wrote_then_notified(&fakes, EditorStep::WroteFor(agent()));
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
        assert_wrote_then_notified(&fakes, EditorStep::Wrote);
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

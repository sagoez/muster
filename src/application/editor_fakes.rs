use std::cell::RefCell;

use super::{
    editor_fixtures::{registration, settings, spec},
    editor_step::EditorStep,
    stack_editor::StackEditor,
};
use crate::domain::{
    config::{
        AppliedChange, AppliedSettings, ConfigError, ImportMode, ImportPlan, ProcessChange,
        ProcessSpec, Registration, SettingChange, Stack,
    },
    coordination::Author,
    port::{SettingsReader, SettingsStore, StackReader, StackStore, StateChangeNotifier},
    project::Project,
    settings::Settings,
    value::{IdleStop, ProcessName, ProjectRoot, StackRevision},
};

/// What a refused write reports.
const REFUSED: &str = "refused";

/// Fakes every port an editor drives over one shared log; `fail` makes every
/// write refuse.
pub(super) struct EditorFakes {
    log: RefCell<Vec<EditorStep>>,
    added: RefCell<Vec<ProcessSpec>>,
    settings: RefCell<Settings>,
    fail: bool,
    /// Whether the project every write lands in is registered already.
    known: bool,
}

impl EditorFakes {
    /// Fakes over [`settings`], refusing every write when `fail`, each
    /// stack write registering its project.
    pub(super) fn new(fail: bool) -> Self {
        Self {
            log: RefCell::new(Vec::new()),
            added: RefCell::new(Vec::new()),
            settings: RefCell::new(settings()),
            fail,
            known: false,
        }
    }

    /// These fakes with the project every write lands in registered
    /// already.
    pub(super) fn over_known_project(self) -> Self {
        Self {
            known: true,
            ..self
        }
    }

    /// An editor driving these fakes as every port.
    pub(super) fn editor(&self) -> StackEditor<'_> {
        StackEditor::builder()
            .stacks(self)
            .settings(self)
            .notifier(self)
            .build()
    }

    /// Every process definition added, in order.
    pub(super) fn added(&self) -> Vec<ProcessSpec> {
        self.added.borrow().clone()
    }

    /// Every step recorded, in order.
    pub(super) fn steps(&self) -> Vec<EditorStep> {
        self.log.borrow().clone()
    }

    /// Records a write, or refuses it when failing.
    fn write(&self) -> Result<(), ConfigError> {
        self.record(EditorStep::Wrote)
    }

    /// Records a stack write for `author`, or refuses it when failing.
    fn write_for(&self, author: &Author) -> Result<(), ConfigError> {
        self.record(EditorStep::WroteFor(author.clone()))
    }

    /// Records `step`, or refuses it when failing.
    fn record(&self, step: EditorStep) -> Result<(), ConfigError> {
        if self.fail {
            return Err(ConfigError::Storage(REFUSED.into()));
        }
        self.log.borrow_mut().push(step);
        Ok(())
    }
}

impl StackReader for EditorFakes {
    fn stack(&self, _root: &ProjectRoot) -> Result<Option<Stack>, ConfigError> {
        Ok(None)
    }

    fn stack_revision(&self, _root: &ProjectRoot) -> Result<Option<StackRevision>, ConfigError> {
        Ok(None)
    }
}

impl StackStore for EditorFakes {
    fn add_process(
        &self,
        _root: &ProjectRoot,
        spec: &ProcessSpec,
        author: &Author,
    ) -> Result<Registration, ConfigError> {
        self.write_for(author)?;
        self.added.borrow_mut().push(spec.clone());
        Ok(registration(!self.known))
    }

    fn change_process(
        &self,
        _root: &ProjectRoot,
        _name: &ProcessName,
        change: &ProcessChange,
        author: &Author,
    ) -> Result<AppliedChange, ConfigError> {
        self.write_for(author)?;
        let changed = spec().with_change(change)?;
        Ok(AppliedChange::builder()
            .changed(changed != spec())
            .spec(changed)
            .build())
    }

    fn remove_process(
        &self,
        _root: &ProjectRoot,
        _name: &ProcessName,
        author: &Author,
    ) -> Result<ProcessSpec, ConfigError> {
        self.write_for(author)?;
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
            .project(registration(!self.known).project().clone())
            .processes(Vec::new())
            .build();
        Ok((
            registration(!self.known),
            stack.plan_import(processes, mode)?,
        ))
    }

    fn set_idle_stop(
        &self,
        _root: &ProjectRoot,
        idle_stop: Option<IdleStop>,
    ) -> Result<Project, ConfigError> {
        self.write()?;
        let project = registration(!self.known).project().clone();
        Ok(Project::builder()
            .root(project.root().clone())
            .name(project.name().clone())
            .idle_stop(idle_stop)
            .build())
    }
}

impl SettingsReader for EditorFakes {
    fn load(&self) -> Result<Settings, ConfigError> {
        Ok(self.settings.borrow().clone())
    }
}

impl SettingsStore for EditorFakes {
    fn apply(
        &self,
        change: &SettingChange,
        _author: &Author,
    ) -> Result<AppliedSettings, ConfigError> {
        self.write()?;
        let before = self.settings.borrow().clone();
        let after = before.clone().with_change(change);
        *self.settings.borrow_mut() = after.clone();
        Ok(AppliedSettings::builder()
            .changed(after != before)
            .settings(after)
            .build())
    }
}

impl StateChangeNotifier for EditorFakes {
    fn project_registered(&self, root: &ProjectRoot) {
        self.log
            .borrow_mut()
            .push(EditorStep::ProjectRegistered(root.clone()));
    }

    fn stack_changed(&self, root: &ProjectRoot) {
        self.log
            .borrow_mut()
            .push(EditorStep::StackChanged(root.clone()));
    }

    fn settings_changed(&self) {
        self.log.borrow_mut().push(EditorStep::SettingsChanged);
    }
}

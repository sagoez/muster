use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::SystemTime,
};

use super::supervisor::testing::test_settings;
use crate::domain::{
    config::{
        AppliedSettings, ConfigError, ImportMode, ImportPlan, ProcessChange, ProcessSpec,
        RecordedEdit, Registration, SettingChange, Stack, StackEdit,
    },
    coordination::Author,
    port::{
        RaisedEdits, RunHistory, SettingsReader, SettingsStore, StackEditLog, StackReader,
        StackStore,
    },
    process::EndedRun,
    project::Project,
    settings::Settings,
    value::{EditId, ProcessName, ProjectName, ProjectRoot, StackRevision},
};

/// Stacks and settings held in memory and shared with the test, which
/// edits them as a writer would before telling the service.
#[derive(Clone)]
pub(super) struct MemoryStore {
    stacks: Arc<Mutex<HashMap<ProjectRoot, Vec<ProcessSpec>>>>,
    /// The writes made so far, and the revision of each registered stack.
    revisions: Arc<Mutex<(i64, HashMap<ProjectRoot, StackRevision>)>>,
    settings: Arc<Mutex<Settings>>,
    /// Every agent edit kept, oldest first, with the project it belongs to.
    edits: Arc<Mutex<Vec<(ProjectRoot, RecordedEdit)>>>,
    /// How each process last ended on its own, by project and process.
    runs: Arc<Mutex<HashMap<(ProjectRoot, ProcessName), EndedRun>>>,
    /// The newest agent edit of each project a service raised.
    raised: Arc<Mutex<HashMap<ProjectRoot, EditId>>>,
}

impl MemoryStore {
    /// An empty store with the default settings.
    pub(super) fn new() -> Self {
        Self {
            stacks: Arc::default(),
            revisions: Arc::default(),
            settings: Arc::new(Mutex::new(test_settings())),
            edits: Arc::default(),
            runs: Arc::default(),
            raised: Arc::default(),
        }
    }

    /// Keeps an agent edit to `process` of `root` reading `message`, made
    /// now, moving the stack's revision with it as the real store's write
    /// does.
    pub(super) fn record_edit(&self, root: &ProjectRoot, process: &str, message: &str) {
        let mut edits = self.edits.lock().unwrap();
        let id = i64::try_from(edits.len()).unwrap() + 1;
        edits.push((
            root.clone(),
            RecordedEdit::builder()
                .id(EditId::new(id))
                .at(SystemTime::now())
                .process(Some(ProcessName::try_new(process).unwrap()))
                .message(message.to_string())
                .build(),
        ));
        drop(edits);
        self.bump(root);
    }

    /// Registers `root` with `processes`, replacing its stack.
    pub(super) fn put(&self, root: &ProjectRoot, processes: Vec<ProcessSpec>) {
        self.stacks.lock().unwrap().insert(root.clone(), processes);
        self.bump(root);
    }

    /// Unregisters `root`.
    pub(super) fn forget(&self, root: &ProjectRoot) {
        self.stacks.lock().unwrap().remove(root);
        self.revisions.lock().unwrap().1.remove(root);
    }

    /// Moves `root`'s stack to the next revision, as every write does.
    pub(super) fn bump(&self, root: &ProjectRoot) {
        let mut revisions = self.revisions.lock().unwrap();
        revisions.0 += 1;
        let next = StackRevision::new(revisions.0);
        revisions.1.insert(root.clone(), next);
    }

    /// The project at `root`, named for its folder.
    fn project(root: &ProjectRoot) -> Project {
        Project::builder()
            .root(root.clone())
            .name(ProjectName::candidates(root).next().unwrap())
            .build()
    }
}

impl StackReader for MemoryStore {
    fn stack(&self, root: &ProjectRoot) -> Result<Option<Stack>, ConfigError> {
        Ok(self.stacks.lock().unwrap().get(root).map(|processes| {
            Stack::builder()
                .project(Self::project(root))
                .processes(processes.clone())
                .build()
        }))
    }

    fn stack_revision(&self, root: &ProjectRoot) -> Result<Option<StackRevision>, ConfigError> {
        Ok(self.revisions.lock().unwrap().1.get(root).copied())
    }
}

impl StackStore for MemoryStore {
    fn add_process(
        &self,
        root: &ProjectRoot,
        spec: &ProcessSpec,
        _author: &Author,
    ) -> Result<Registration, ConfigError> {
        let mut stacks = self.stacks.lock().unwrap();
        let newly_registered = !stacks.contains_key(root);
        stacks.entry(root.clone()).or_default().push(spec.clone());
        self.bump(root);
        Ok(Registration::builder()
            .project(Self::project(root))
            .newly_registered(newly_registered)
            .build())
    }

    fn change_process(
        &self,
        root: &ProjectRoot,
        name: &ProcessName,
        change: &ProcessChange,
        _author: &Author,
    ) -> Result<ProcessSpec, ConfigError> {
        let mut stacks = self.stacks.lock().unwrap();
        let spec = stacks
            .get_mut(root)
            .and_then(|processes| processes.iter_mut().find(|spec| spec.name() == name))
            .ok_or_else(|| ConfigError::Storage(name.to_string().into()))?;
        *spec = spec.clone().with_change(change)?;
        let changed = spec.clone();
        self.bump(root);
        Ok(changed)
    }

    fn remove_process(
        &self,
        root: &ProjectRoot,
        name: &ProcessName,
        _author: &Author,
    ) -> Result<ProcessSpec, ConfigError> {
        let mut stacks = self.stacks.lock().unwrap();
        let processes = stacks
            .get_mut(root)
            .ok_or_else(|| ConfigError::Storage(name.to_string().into()))?;
        let index = processes
            .iter()
            .position(|spec| spec.name() == name)
            .ok_or_else(|| ConfigError::Storage(name.to_string().into()))?;
        let removed = processes.remove(index);
        self.bump(root);
        Ok(removed)
    }

    fn import(
        &self,
        _root: &ProjectRoot,
        _processes: &[ProcessSpec],
        _mode: ImportMode,
    ) -> Result<(Registration, ImportPlan), ConfigError> {
        Err(ConfigError::Storage("not used by these tests".into()))
    }
}

impl StackEditLog for MemoryStore {
    fn edits(&self, root: &ProjectRoot) -> Result<Vec<RecordedEdit>, ConfigError> {
        Ok(self
            .edits
            .lock()
            .unwrap()
            .iter()
            .filter(|(project, _)| project == root)
            .map(|(_, edit)| edit.clone())
            .collect())
    }
}

impl RunHistory for MemoryStore {
    fn ended_runs(&self, root: &ProjectRoot) -> Result<Vec<EndedRun>, ConfigError> {
        Ok(self
            .runs
            .lock()
            .unwrap()
            .iter()
            .filter(|((project, _), _)| project == root)
            .map(|(_, run)| run.clone())
            .collect())
    }

    fn record_end(&self, root: &ProjectRoot, run: &EndedRun) -> Result<(), ConfigError> {
        self.runs
            .lock()
            .unwrap()
            .insert((root.clone(), run.process().clone()), run.clone());
        Ok(())
    }

    fn clear_end(&self, root: &ProjectRoot, process: &ProcessName) -> Result<(), ConfigError> {
        self.runs
            .lock()
            .unwrap()
            .remove(&(root.clone(), process.clone()));
        Ok(())
    }
}

impl RaisedEdits for MemoryStore {
    fn raised_through(&self, root: &ProjectRoot) -> Result<Option<EditId>, ConfigError> {
        Ok(self.raised.lock().unwrap().get(root).copied())
    }

    fn mark_raised(&self, root: &ProjectRoot, edit: EditId) -> Result<(), ConfigError> {
        let mut raised = self.raised.lock().unwrap();
        let mark = raised.entry(root.clone()).or_insert(edit);
        *mark = (*mark).max(edit);
        Ok(())
    }
}

impl SettingsReader for MemoryStore {
    fn load(&self) -> Result<Settings, ConfigError> {
        Ok(self.settings.lock().unwrap().clone())
    }
}

impl SettingsStore for MemoryStore {
    fn apply(
        &self,
        change: &SettingChange,
        author: &Author,
    ) -> Result<AppliedSettings, ConfigError> {
        let mut settings = self.settings.lock().unwrap();
        let before = settings.clone();
        *settings = settings.clone().with_change(change);
        let key = change.key();
        let edit = StackEdit::Configured {
            author: author.clone(),
            key,
            value: settings.value_text(key),
        };
        let changed = *settings != before;
        if changed && edit.is_logged() {
            let roots: Vec<ProjectRoot> = self.stacks.lock().unwrap().keys().cloned().collect();
            let mut edits = self.edits.lock().unwrap();
            for root in roots {
                let id = i64::try_from(edits.len()).unwrap() + 1;
                edits.push((
                    root,
                    RecordedEdit::builder()
                        .id(EditId::new(id))
                        .at(SystemTime::UNIX_EPOCH)
                        .process(None)
                        .message(edit.to_string())
                        .build(),
                ));
            }
        }
        Ok(AppliedSettings::builder()
            .settings(settings.clone())
            .changed(changed)
            .build())
    }
}

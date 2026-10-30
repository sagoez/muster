use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::SystemTime,
};

use super::supervisor::testing::test_settings;
use crate::domain::{
    config::{
        AppliedChange, AppliedSettings, ConfigError, ImportMode, ImportPlan, ProcessChange,
        ProcessSpec, RecordedEdit, Registration, SettingChange, Stack, StackEdit,
    },
    coordination::Author,
    occupancy::PendingIdleStop,
    port::{
        PendingIdleStops, RaisedEdits, RunHistory, SettingsReader, SettingsStore, StackEditLog,
        StackReader, StackStore,
    },
    process::EndedRun,
    project::Project,
    settings::Settings,
    value::{EditId, IdleStop, ProcessName, ProjectName, ProjectRoot, StackRevision},
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
    /// Each project's own idle stop, when it sets one.
    idle_stops: Arc<Mutex<HashMap<ProjectRoot, IdleStop>>>,
    /// When the last shell left each project whose idle stop is pending.
    pending_idle: Arc<Mutex<HashMap<ProjectRoot, SystemTime>>>,
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
            idle_stops: Arc::default(),
            pending_idle: Arc::default(),
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
        self.idle_stops.lock().unwrap().remove(root);
    }

    /// Moves `root`'s stack to the next revision, as every write does.
    pub(super) fn bump(&self, root: &ProjectRoot) {
        let mut revisions = self.revisions.lock().unwrap();
        revisions.0 += 1;
        let next = StackRevision::new(revisions.0);
        revisions.1.insert(root.clone(), next);
    }

    /// The project at `root`, named for its folder, with its own idle stop.
    fn project(&self, root: &ProjectRoot) -> Project {
        Project::builder()
            .root(root.clone())
            .name(ProjectName::candidates(root).next().unwrap())
            .idle_stop(self.idle_stops.lock().unwrap().get(root).copied())
            .build()
    }
}

impl StackReader for MemoryStore {
    fn stack(&self, root: &ProjectRoot) -> Result<Option<Stack>, ConfigError> {
        Ok(self.stacks.lock().unwrap().get(root).map(|processes| {
            Stack::builder()
                .project(self.project(root))
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
            .project(self.project(root))
            .newly_registered(newly_registered)
            .build())
    }

    fn change_process(
        &self,
        root: &ProjectRoot,
        name: &ProcessName,
        change: &ProcessChange,
        _author: &Author,
    ) -> Result<AppliedChange, ConfigError> {
        let mut stacks = self.stacks.lock().unwrap();
        let spec = stacks
            .get_mut(root)
            .and_then(|processes| processes.iter_mut().find(|spec| spec.name() == name))
            .ok_or_else(|| ConfigError::Storage(name.to_string().into()))?;
        let changed = spec.clone().with_change(change)?;
        let applied = AppliedChange::builder()
            .changed(changed != *spec)
            .spec(changed.clone())
            .build();
        *spec = changed;
        self.bump(root);
        Ok(applied)
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

    fn set_idle_stop(
        &self,
        root: &ProjectRoot,
        idle_stop: Option<IdleStop>,
    ) -> Result<Project, ConfigError> {
        if !self.stacks.lock().unwrap().contains_key(root) {
            return Err(ConfigError::UnknownProject {
                root: root.as_ref().clone(),
            });
        }
        let mut idle_stops = self.idle_stops.lock().unwrap();
        match idle_stop {
            Some(idle_stop) => idle_stops.insert(root.clone(), idle_stop),
            None => idle_stops.remove(root),
        };
        drop(idle_stops);
        self.bump(root);
        Ok(self.project(root))
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

impl PendingIdleStops for MemoryStore {
    fn pending_idle_stops(&self) -> Result<Vec<PendingIdleStop>, ConfigError> {
        let mut stops: Vec<PendingIdleStop> = self
            .pending_idle
            .lock()
            .unwrap()
            .iter()
            .map(|(project, left_at)| {
                PendingIdleStop::builder()
                    .project(project.clone())
                    .left_at(*left_at)
                    .build()
            })
            .collect();
        stops.sort_by(|a, b| a.project().cmp(b.project()));
        Ok(stops)
    }

    fn arm_idle_stop(&self, stop: &PendingIdleStop) -> Result<(), ConfigError> {
        if self.stacks.lock().unwrap().contains_key(stop.project()) {
            self.pending_idle
                .lock()
                .unwrap()
                .insert(stop.project().clone(), *stop.left_at());
        }
        Ok(())
    }

    fn clear_idle_stop(&self, root: &ProjectRoot) -> Result<(), ConfigError> {
        self.pending_idle.lock().unwrap().remove(root);
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

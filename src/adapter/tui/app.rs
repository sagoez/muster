use std::{
    collections::{HashMap, HashSet},
    mem,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, unbounded};
use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
};
use vt100::{Parser, Screen};

use super::{
    activity::ActivityTracker,
    completion_generation::CompletionGeneration,
    event::{ChannelOutputSink, RuntimeEvent},
    form::{Field, Form, FormOutcome},
    input,
    signal::{Signal, SignalReader},
    spawn_generation::SpawnGeneration,
    widget::{
        confirm, empty_state, form, help, sidebar, status_bar, status_bar::StatusContext, switcher,
        terminal_pane,
    },
};
use crate::{
    adapter::path,
    application::Workspace,
    constants::APP_NAME,
    domain::{
        config::{ConfigError, ProcessSpec, WorkspaceConfig},
        notification::{Notification, NotificationId, NotificationScope},
        port::{
            ConfigWatcher, Notifier, PathCompleter, ProcessHandle, ProcessRunner, ProjectRegistry,
            SettingsStore,
        },
        process::{ActivityState, ExitIntent, Process, ProcessKind, ProcessState, RestartPolicy},
        project::Project,
        pty::{ExitOutcome, ProcessOutput, PtySize, SpawnRequest},
        settings::Settings,
        value::{Cols, CommandLine, Description, PaneId, ProcessName, ProjectName, Rows},
    },
};

/// Sidebar width, in columns.
const SIDEBAR_WIDTH: u16 = 32;
/// Status bar height, in rows.
const STATUS_BAR_HEIGHT: u16 = 1;
/// Terminal-pane border thickness, per side.
const BORDER_THICKNESS: u16 = 1;
/// Scrollback lines retained per pane.
const SCROLLBACK_LINES: usize = 1000;
/// Notice shown for a bare bell notification that carried no text of its own.
const AWAITING_INPUT_NOTICE: &str = "awaiting input";
/// Minimum gap between bell notifications from one pane, absorbing a burst (e.g.
/// shell tab-completion) into a single alert.
const BELL_THROTTLE: Duration = Duration::from_secs(3);
/// Leader key (pressed with Control) that begins a command chord.
const LEADER_KEY: char = 'a';
/// Minimum PTY dimension. vt100 underflows on some sequences below a couple of
/// cells (e.g. a wide glyph in a 1-column grid), so a pane is never sized smaller
/// than this even when the real area is tiny or zero.
const MIN_PANE_DIMENSION: u16 = 8;
/// Delay before the first automatic restart of a failing process.
const RESTART_BACKOFF_BASE: Duration = Duration::from_millis(200);
/// Upper bound on the automatic-restart delay.
const RESTART_BACKOFF_MAX: Duration = Duration::from_secs(5);
/// Cap on the backoff exponent (delay = base * 2^exp, clamped to the max).
const RESTART_BACKOFF_MAX_EXP: u32 = 5;
/// A process that ran at least this long counts as stable; its next restart
/// resets the backoff so a healthy long-lived process is not penalized.
const RESTART_STABLE_RUN: Duration = Duration::from_secs(10);
/// Grace allowed for a manually stopped command to handle SIGTERM, emit final
/// output, and exit before Muster escalates to a hard kill.
const COMMAND_STOP_GRACE: Duration = Duration::from_secs(3);
/// Title of the save-current-project form.
const SAVE_PROJECT_TITLE: &str = "Save project";
/// Title of the new-project form.
const NEW_PROJECT_TITLE: &str = "New project";
/// Label of a project-name field.
const NAME_FIELD: &str = "Name";
/// Label of a project-folder field.
const FOLDER_FIELD: &str = "Folder";
/// Workspace config file name created inside a new project's folder.
const PROJECT_CONFIG_FILE: &str = "muster.yml";
/// Name of the starter terminal created for a new project.
const STARTER_TERMINAL: &str = "Terminal";
/// Title of the add-process form.
const ADD_PROCESS_TITLE: &str = "Add process";
/// Label of a process-kind field.
const KIND_FIELD: &str = "Kind";
/// Label of a command field.
const COMMAND_FIELD: &str = "Command";
/// Process-kind option value for an agent.
const KIND_AGENT: &str = "agent";
/// Process-kind option value for a terminal.
const KIND_TERMINAL: &str = "terminal";
/// Process-kind option value for a command.
const KIND_COMMAND: &str = "command";
/// Process-kind options offered when adding a process.
const KIND_OPTIONS: [&str; 3] = [KIND_AGENT, KIND_TERMINAL, KIND_COMMAND];
/// Shown when a project's config file cannot be written.
const WORKSPACE_SAVE_ERROR: &str = "could not write the project config";
/// Shown when autostart cannot persist because the process has no config entry
/// (for example a process left running after its spec was removed).
const AUTOSTART_UNTRACKED: &str = "autostart unchanged: process is not in the config";
/// Confirmation shown when desktop notifications are toggled on.
const DESKTOP_NOTIFICATIONS_ON: &str = "desktop notifications on";
/// Confirmation shown when desktop notifications are toggled off.
const DESKTOP_NOTIFICATIONS_OFF: &str = "desktop notifications off";
/// Shown when the settings file cannot be written.
const SETTINGS_SAVE_ERROR: &str = "could not save settings";
/// Shown when settings could not be loaded and must not be overwritten.
const SETTINGS_LOAD_ERROR: &str = "could not load settings; file left unchanged";
/// Shown when settings have not yet been wired by the composition root.
const SETTINGS_UNAVAILABLE: &str = "settings are unavailable";
/// Shown when removal is asked of the launched project's synthetic row, which
/// has no registry entry to remove.
const CANNOT_REMOVE_LAUNCHED: &str = "this project is not saved, so there is nothing to remove";
/// Shown when the registry file cannot be written.
const REGISTRY_SAVE_ERROR: &str = "could not save the project registry";
/// Confirmation shown before overwriting an existing config.
const OVERWRITE_CONFIRM: &str = "A muster.yml already exists in that folder.";
/// Title of the overwrite confirmation.
const OVERWRITE_TITLE: &str = "Overwrite?";
/// Accept-action verb for the overwrite confirmation.
const OVERWRITE_VERB: &str = "overwrite";
/// Title of the project-removal confirmation.
const REMOVE_PROJECT_TITLE: &str = "Remove project?";
/// Accept-action verb for the project-removal confirmation.
const REMOVE_PROJECT_VERB: &str = "remove";

/// Which region currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    /// The sidebar list is active: keys navigate and act on processes.
    Sidebar,
    /// A terminal is attached: keys pass through to its PTY.
    Terminal,
    /// A terminal is attached and the next key completes a leader command.
    Leader,
}

/// Whether a pane still belongs to the current config.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ConfigMembership {
    /// The pane is represented by a current process specification.
    #[default]
    Tracked,
    /// The live pane was removed from config and must disappear when it exits.
    RetireOnExit,
}

/// Whether the project Muster launched with has a persisted registry entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum LaunchedProjectMembership {
    /// The launched config is represented by a saved project entry.
    #[default]
    Registered,
    /// The launched config appears only as a temporary sidebar row.
    Synthetic,
}

/// One managed pane: its VT parser and, while alive, a live process handle. The
/// parser outlives the handle so a finished process keeps its last screen.
struct Pane {
    parser: Parser,
    /// Unique scope for Kitty identifiers emitted by this terminal lifetime.
    notification_scope: NotificationScope,
    /// Decodes notification and progress signals from the same output stream the
    /// vt100 parser renders.
    signals: SignalReader,
    /// Evidence governing inferred working, idle, and attention transitions.
    activity: ActivityTracker,
    /// Last bell accepted from this terminal lifetime, for burst throttling.
    last_bell: Option<Instant>,
    handle: Option<Box<dyn ProcessHandle>>,
    started_at: Instant,
    exit_intent: ExitIntent,
    config_membership: ConfigMembership,
}

/// Result of loading cross-workspace settings. A failure remains distinct from
/// the not-yet-wired state so a toggle never replaces an unreadable file.
enum SettingsState {
    /// The composition root has not provided a settings store yet.
    Unloaded,
    /// Settings loaded successfully and retain the store required for updates.
    Loaded {
        settings: Settings,
        store: Box<dyn SettingsStore>,
    },
    /// An existing settings file could not be read or parsed.
    LoadFailed(ConfigError),
}

/// Open state of the project-switcher overlay: the loaded project list, the
/// highlighted row, and any switch failure to surface.
struct Switcher {
    projects: Vec<Project>,
    selected: usize,
    current: Option<usize>,
    error: Option<String>,
    /// The highlighted project's processes, cached so render does no I/O.
    preview: Vec<(ProcessKind, String)>,
}

impl Switcher {
    /// Loads `project`'s processes for a preview, or returns an empty list when
    /// there is no project or its config cannot be read.
    fn preview(
        registry: &dyn ProjectRegistry,
        project: Option<&Project>,
    ) -> Vec<(ProcessKind, String)> {
        project
            .and_then(|project| registry.workspace(project.config()).ok())
            .map(|config| Self::config_preview(&config))
            .unwrap_or_default()
    }

    /// Converts a project's process sections to the kind/name rows it previews.
    fn config_preview(config: &WorkspaceConfig) -> Vec<(ProcessKind, String)> {
        [
            (ProcessKind::Agent, config.agents()),
            (ProcessKind::Terminal, config.terminals()),
            (ProcessKind::Command, config.commands()),
        ]
        .into_iter()
        .flat_map(|(kind, specs)| {
            specs
                .iter()
                .map(move |spec| (kind, spec.name().as_ref().to_string()))
        })
        .collect()
    }
}

/// A project switch deferred until the current children have exited, so the new
/// workspace never contends with the old for ports or other resources.
struct PendingSwitch {
    config: WorkspaceConfig,
    config_path: PathBuf,
    waiting: HashSet<PaneId>,
}

/// An open input form and what its values do on submit.
struct FormModal {
    form: Form,
    intent: FormIntent,
    error: Option<String>,
}

/// An input form and the switcher it should reveal when closed.
struct FormOverlay {
    modal: FormModal,
    switcher: Option<Switcher>,
}

/// The one legal modal UI state. Variants retain only the exact background
/// required by their cancel and failure transitions.
enum Overlay {
    Switcher(Switcher),
    Form(FormOverlay),
    ConfirmOverwrite {
        form: FormOverlay,
        name: ProjectName,
        config_path: PathBuf,
    },
    ConfirmRemoval {
        message: String,
        config_path: PathBuf,
    },
    Help,
}

impl Overlay {
    /// Returns the switcher in this overlay stack, if present.
    fn switcher(&self) -> Option<&Switcher> {
        match self {
            Self::Switcher(switcher) => Some(switcher),
            Self::Form(form) | Self::ConfirmOverwrite { form, .. } => form.switcher.as_ref(),
            Self::ConfirmRemoval { .. } | Self::Help => None,
        }
    }

    /// Returns the switcher in this overlay stack mutably, if present.
    fn switcher_mut(&mut self) -> Option<&mut Switcher> {
        match self {
            Self::Switcher(switcher) => Some(switcher),
            Self::Form(form) | Self::ConfirmOverwrite { form, .. } => form.switcher.as_mut(),
            Self::ConfirmRemoval { .. } | Self::Help => None,
        }
    }

    /// Returns the form in this overlay stack, if present.
    fn form(&self) -> Option<&FormModal> {
        match self {
            Self::Form(form) | Self::ConfirmOverwrite { form, .. } => Some(&form.modal),
            Self::Switcher(_) | Self::ConfirmRemoval { .. } | Self::Help => None,
        }
    }

    /// Returns the form in this overlay stack mutably, if present.
    fn form_mut(&mut self) -> Option<&mut FormModal> {
        match self {
            Self::Form(form) | Self::ConfirmOverwrite { form, .. } => Some(&mut form.modal),
            Self::Switcher(_) | Self::ConfirmRemoval { .. } | Self::Help => None,
        }
    }

    /// Renders this overlay and any explicit background it retains.
    fn render(&self, frame: &mut Frame) {
        let area = frame.area();
        match self {
            Self::Switcher(switcher) => Self::render_switcher(frame, area, switcher),
            Self::Form(form_overlay) => {
                Self::render_form(frame, area, form_overlay);
            },
            Self::ConfirmOverwrite { form, .. } => {
                Self::render_form(frame, area, form);
                confirm::render(
                    frame,
                    area,
                    OVERWRITE_TITLE,
                    OVERWRITE_CONFIRM,
                    OVERWRITE_VERB,
                );
            },
            Self::ConfirmRemoval { message, .. } => confirm::render(
                frame,
                area,
                REMOVE_PROJECT_TITLE,
                message,
                REMOVE_PROJECT_VERB,
            ),
            Self::Help => help::render(frame, area),
        }
    }

    /// Renders a form with its retained switcher background, if any.
    fn render_form(frame: &mut Frame, area: Rect, form_overlay: &FormOverlay) {
        if let Some(switcher) = &form_overlay.switcher {
            Self::render_switcher(frame, area, switcher);
        }
        let modal = &form_overlay.modal;
        form::render(frame, area, &modal.form, modal.error.as_deref());
    }

    /// Renders a project switcher layer.
    fn render_switcher(frame: &mut Frame, area: Rect, switcher: &Switcher) {
        switcher::render(
            frame,
            area,
            &switcher.projects,
            switcher.selected,
            switcher.error.as_deref(),
            switcher.current,
            &switcher.preview,
        );
    }
}

/// What a submitted form should do.
#[derive(Clone, Copy)]
enum FormIntent {
    /// Register the current workspace under the typed name.
    SaveCurrentProject,
    /// Create a new project: write a starter config at the folder and register it.
    NewProject,
    /// Add a process to the current workspace and reload it.
    AddProcess,
}

/// The running TUI application: workspace state plus the live panes.
pub struct App {
    workspace: Workspace,
    runner: Box<dyn ProcessRunner>,
    events: Sender<RuntimeEvent>,
    panes: HashMap<PaneId, Pane>,
    restart_attempts: HashMap<PaneId, u32>,
    generations: HashMap<PaneId, SpawnGeneration>,
    /// Monotonic source for terminal-lifetime notification scopes.
    next_notification_scope: NotificationScope,
    pane_size: PtySize,
    focus: Focus,
    running: bool,
    registry: Box<dyn ProjectRegistry>,
    completion_mode: CompletionMode,
    watcher: Option<Box<dyn ConfigWatcher>>,
    current_config: Option<PathBuf>,
    /// The config muster launched with, kept reachable in the tree even when it
    /// was never saved, so switching away from it is never a dead end.
    launched_config: PathBuf,
    /// Registry membership of the config Muster launched with.
    launched_project_membership: LaunchedProjectMembership,
    projects: Vec<Project>,
    /// When set, the sidebar selection is on the Nth other-project row rather
    /// than on a process in the active project.
    project_cursor: Option<usize>,
    overlay: Option<Overlay>,
    pending_switch: Option<PendingSwitch>,
    /// Out-of-band notification delivery (desktop), injected by the composition
    /// root via [`Self::set_notifier`]. `None` until then: notifications stay
    /// in-app, and the App itself never names a concrete notifier adapter.
    notifier: Option<Box<dyn Notifier>>,
    /// Cross-workspace settings and their load result.
    settings: SettingsState,
    /// A transient one-line message shown in the status bar until the next key,
    /// used for failures that have no open overlay to report into.
    notice: Option<String>,
}

/// A pending autocomplete request handed to the completion worker: the newest
/// generation wins, so slow filesystem reads never clobber later edits.
struct CompletionRequest {
    generation: CompletionGeneration,
    partial: String,
}

/// How path completions are currently evaluated.
enum CompletionMode {
    /// Complete synchronously until the runtime installs its worker.
    Inline(Box<dyn PathCompleter + Send>),
    /// Dispatch generation-tagged requests to the background worker.
    Worker {
        requests: Sender<CompletionRequest>,
        generation: CompletionGeneration,
    },
}

impl App {
    /// Creates the app sized to `area`, ready for `start`.
    pub fn new(
        workspace: Workspace,
        runner: Box<dyn ProcessRunner>,
        events: Sender<RuntimeEvent>,
        area: Rect,
        completer: Box<dyn PathCompleter + Send>,
        registry: Box<dyn ProjectRegistry>,
        current_config: PathBuf,
    ) -> Self {
        Self {
            workspace,
            runner,
            events,
            panes: HashMap::new(),
            restart_attempts: HashMap::new(),
            generations: HashMap::new(),
            next_notification_scope: NotificationScope::new(0),
            pane_size: pane_size_of(area),
            focus: Focus::Sidebar,
            running: true,
            registry,
            completion_mode: CompletionMode::Inline(completer),
            watcher: None,
            launched_config: current_config.clone(),
            launched_project_membership: LaunchedProjectMembership::Registered,
            current_config: Some(current_config),
            projects: Vec::new(),
            project_cursor: None,
            overlay: None,
            pending_switch: None,
            notifier: None,
            settings: SettingsState::Unloaded,
            notice: None,
        }
    }

    /// Wires the notifier used for out-of-band (desktop) notifications. The
    /// composition root calls this; without it, notifications stay in-app only.
    pub fn set_notifier(&mut self, notifier: Box<dyn Notifier>) {
        self.notifier = Some(notifier);
    }

    /// Wires the settings store and loads the current settings from it. A load
    /// failure is retained so later toggles cannot overwrite the unreadable file.
    pub fn set_settings_store(&mut self, store: Box<dyn SettingsStore>) {
        self.settings = match store.load() {
            Ok(settings) => SettingsState::Loaded { settings, store },
            Err(error) => {
                self.notice = Some(format!("{SETTINGS_LOAD_ERROR}: {error}"));
                SettingsState::LoadFailed(error)
            },
        };
    }

    /// Whether desktop notifications are currently enabled. Unavailable or
    /// invalid settings count as off, so nothing leaves the machine unexpectedly.
    fn desktop_notifications_enabled(&self) -> bool {
        matches!(
            &self.settings,
            SettingsState::Loaded { settings, .. } if *settings.desktop_notifications()
        )
    }

    /// Flips the desktop-notifications setting and persists it, applying the
    /// change only when the write succeeds so the toggle reflects what is saved.
    fn toggle_desktop_notifications(&mut self) {
        match &mut self.settings {
            SettingsState::Loaded { settings, store } => {
                let enabled = !*settings.desktop_notifications();
                let updated = settings.clone().with_desktop_notifications(enabled);
                if store.save(&updated).is_ok() {
                    *settings = updated;
                    self.notice = Some(
                        if enabled {
                            DESKTOP_NOTIFICATIONS_ON
                        } else {
                            DESKTOP_NOTIFICATIONS_OFF
                        }
                        .to_string(),
                    );
                } else {
                    self.notice = Some(SETTINGS_SAVE_ERROR.to_string());
                }
            },
            SettingsState::LoadFailed(error) => {
                self.notice = Some(format!("{SETTINGS_LOAD_ERROR}: {error}"));
            },
            SettingsState::Unloaded => {
                self.notice = Some(SETTINGS_UNAVAILABLE.to_string());
            },
        }
    }

    /// Returns the project switcher in the overlay stack, if present.
    fn switcher(&self) -> Option<&Switcher> {
        self.overlay.as_ref().and_then(Overlay::switcher)
    }

    /// Returns the project switcher in the overlay stack mutably, if present.
    fn switcher_mut(&mut self) -> Option<&mut Switcher> {
        self.overlay.as_mut().and_then(Overlay::switcher_mut)
    }

    /// Returns the open form, including one retained behind confirmation.
    fn form(&self) -> Option<&FormModal> {
        self.overlay.as_ref().and_then(Overlay::form)
    }

    /// Returns the active form mutably. A retained confirmation background is
    /// included so operation failures can be reported after restoring it.
    fn form_mut(&mut self) -> Option<&mut FormModal> {
        self.overlay.as_mut().and_then(Overlay::form_mut)
    }

    /// Closes the active overlay and restores its explicitly retained background.
    fn close_overlay(&mut self) {
        let current = self.overlay.take();
        self.overlay = match current {
            Some(Overlay::Form(form)) => form.switcher.map(Overlay::Switcher),
            Some(Overlay::ConfirmOverwrite { form, .. }) => Some(Overlay::Form(form)),
            Some(Overlay::Switcher(_) | Overlay::ConfirmRemoval { .. } | Overlay::Help) | None => {
                None
            },
        };
    }

    /// Whether the event loop should keep running.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Spawns every configured process.
    pub fn start(&mut self) {
        for (pane, command, cwd) in self.spawn_list() {
            self.spawn(pane, command, cwd);
        }
        self.refresh_projects();
    }

    /// Reloads the cached registered-project list shown in the sidebar tree.
    fn refresh_projects(&mut self) {
        let mut projects = self.registry.projects().unwrap_or_default();
        // Keep the launched project reachable even if it was never saved, so
        // switching away from it is never a one-way trip.
        let launched = self.launched_config.clone();
        let registered = projects
            .iter()
            .any(|project| path::normalize(project.config()) == path::normalize(&launched));
        self.launched_project_membership = LaunchedProjectMembership::Registered;
        if !registered && let Ok(name) = ProjectName::try_new(label_from_config(&launched)) {
            projects.insert(0, Project::builder().name(name).config(launched).build());
            self.launched_project_membership = LaunchedProjectMembership::Synthetic;
        }
        self.projects = projects;
    }

    /// Moves directory completion onto a worker thread so a slow or hung
    /// filesystem read never blocks the event loop. Candidates return as
    /// [`RuntimeEvent::Completions`]; without this the completer runs inline.
    pub fn spawn_completion_worker(&mut self) {
        let (requests_tx, requests_rx) = unbounded::<CompletionRequest>();
        let previous = mem::replace(&mut self.completion_mode, CompletionMode::Worker {
            requests: requests_tx,
            generation: CompletionGeneration::initial(),
        });
        match previous {
            CompletionMode::Inline(completer) => {
                let events = self.events.clone();
                thread::spawn(move || completion_worker(completer, requests_rx, events));
            },
            worker @ CompletionMode::Worker { .. } => {
                self.completion_mode = worker;
            },
        }
    }

    /// Installs the config watcher and points it at the active project, so an
    /// external edit or a `muster` CLI append shows up without a restart.
    pub fn set_config_watcher(&mut self, watcher: Box<dyn ConfigWatcher>) {
        self.watcher = Some(watcher);
        self.rewatch_config();
    }

    /// Re-points the config watcher at the active project's config, if both are
    /// present.
    fn rewatch_config(&mut self) {
        if let (Some(watcher), Some(config)) = (self.watcher.as_mut(), self.current_config.as_ref())
        {
            watcher.watch(config);
        }
    }

    /// Reconciles the workspace after the active project's config changed on
    /// disk. Newly-added processes appear (stopped, ready to start); existing
    /// processes keep running untouched, so nothing is bounced and a command
    /// already launched via `muster run` is not started a second time.
    pub fn handle_config_changed(&mut self, path: PathBuf) {
        let Some(current) = self.current_config.clone() else {
            return;
        };
        if path::normalize(&current) != path::normalize(&path) {
            return;
        }
        let Ok(config) = self.registry.workspace(&current) else {
            return;
        };
        self.reconcile_config(&config);
    }

    /// Adds any process present in `config` but not yet in the workspace, as a
    /// pending process with a fresh pane id.
    fn reconcile_config(&mut self, config: &WorkspaceConfig) {
        // Match by the full specification (kind, name, command, working dir,
        // description), not name alone: same-named specs with different commands
        // are distinct, and names are not unique, so count occurrences.
        let sections = [
            (ProcessKind::Agent, config.agents()),
            (ProcessKind::Terminal, config.terminals()),
            (ProcessKind::Command, config.commands()),
        ];
        let spec_identity = |kind: ProcessKind, spec: &ProcessSpec| {
            (
                kind,
                spec.name().clone(),
                spec.command().clone(),
                spec.working_dir().clone(),
                spec.description().clone(),
                spec.restart_policy(),
                spec.should_autostart(kind),
            )
        };
        let mut config_counts = HashMap::new();
        for (kind, specs) in sections {
            for spec in specs {
                *config_counts
                    .entry(spec_identity(kind, spec))
                    .or_insert(0_usize) += 1;
            }
        }

        // Keep an existing process when it still matches the config, or when it
        // is live (running work is never dropped from under the user); a stopped
        // process the config no longer lists is removed. This reflects external
        // removals and modifications, not just additions.
        let mut kept = Vec::new();
        let mut removed = Vec::new();
        let mut next_id = 0;
        for process in self.workspace.processes() {
            next_id = next_id.max((*process.id()).into_inner() + 1);
            let identity = (
                *process.kind(),
                process.name().clone(),
                process.command().clone(),
                process.working_dir().clone(),
                process.description().clone(),
                *process.restart(),
                *process.autostart(),
            );
            let matched = config_counts
                .get_mut(&identity)
                .filter(|count| **count > 0)
                .map(|count| *count -= 1)
                .is_some();
            let pane = *process.id();
            let live = self
                .panes
                .get(&pane)
                .is_some_and(|pane| pane.handle.is_some());
            if matched {
                if let Some(target) = self.panes.get_mut(&pane) {
                    target.config_membership = ConfigMembership::Tracked;
                }
                kept.push(process.clone());
            } else if live {
                // Still running but gone from the config: keep it, and mark it to
                // be retired once it exits instead of restart-looping.
                if let Some(target) = self.panes.get_mut(&pane) {
                    target.config_membership = ConfigMembership::RetireOnExit;
                }
                kept.push(process.clone());
            } else {
                removed.push(pane);
            }
        }

        // Config specs that matched no existing process are new: add them stopped.
        for (kind, specs) in sections {
            for spec in specs {
                if let Some(count) = config_counts
                    .get_mut(&spec_identity(kind, spec))
                    .filter(|count| **count > 0)
                {
                    *count -= 1;
                    kept.push(spec.to_process(PaneId::new(next_id), kind));
                    next_id += 1;
                }
            }
        }

        for pane in removed {
            self.panes.remove(&pane);
            self.generations.remove(&pane);
            self.restart_attempts.remove(&pane);
        }

        // Rebuild the workspace, keeping the selection on the same process when it
        // survives, else clamping to the first.
        let selected = self
            .workspace
            .selected_process()
            .map(|process| *process.id())
            .and_then(|pane| kept.iter().position(|process| *process.id() == pane))
            .unwrap_or(0);
        self.workspace = Workspace::builder()
            .processes(kept)
            .selected_index(selected)
            .build();
    }

    /// Kills every live process; called during shutdown.
    pub fn shutdown(&mut self) {
        for pane in self.panes.values_mut() {
            if let Some(handle) = pane.handle.as_mut() {
                let _ = handle.kill();
            }
        }
    }

    /// Routes a crossterm input event.
    pub fn handle_input(&mut self, event: CrosstermEvent) {
        match event {
            CrosstermEvent::Key(key) => self.handle_key(key),
            CrosstermEvent::Resize(width, height) => self.resize(Rect::new(0, 0, width, height)),
            _ => {},
        }
    }

    /// Applies a process output event, ignoring output from a superseded spawn
    /// generation (e.g. late chunks from a previous child in a restarted pane).
    pub fn handle_output(
        &mut self,
        pane: PaneId,
        generation: SpawnGeneration,
        output: ProcessOutput,
    ) {
        if self.generations.get(&pane) != Some(&generation) {
            return;
        }
        match output {
            ProcessOutput::Chunk(bytes) => {
                let Some(target) = self.panes.get_mut(&pane) else {
                    return;
                };
                let signals = target.signals.read(&bytes);
                target.parser.process(&bytes);
                // Apply signals in stream order, so the final activity reflects
                // the last event in the chunk rather than an assumed one: a bell
                // followed by more output ends working, not awaiting input.
                for signal in signals {
                    self.apply_signal(pane, signal);
                }
            },
            ProcessOutput::Exited(outcome) => self.handle_exit(pane, outcome),
        }
    }

    /// Applies one decoded terminal signal to `pane`: output or in-progress work
    /// marks it working; a notification or completed progress marks it awaiting
    /// input, and a notification also raises an alert.
    fn apply_signal(&mut self, pane: PaneId, signal: Signal) {
        match signal {
            Signal::Output => {
                let activity = self
                    .panes
                    .get_mut(&pane)
                    .map(|target| target.activity.observe_output(Instant::now()));
                if let Some(activity) = activity {
                    self.workspace.set_activity(pane, activity);
                }
            },
            Signal::Progress(active) => {
                let activity = self
                    .panes
                    .get_mut(&pane)
                    .map(|target| target.activity.observe_progress(active));
                if let Some(activity) = activity {
                    self.workspace.set_activity(pane, activity);
                }
            },
            Signal::Notify {
                identifier,
                title,
                body,
            } => {
                if let Some(target) = self.panes.get_mut(&pane) {
                    let activity = target.activity.observe_attention();
                    self.workspace.set_activity(pane, activity);
                }
                self.raise_notification(pane, identifier, title, body);
            },
            Signal::Close { identifier } => self.close_notification(pane, &identifier),
        }
    }

    /// Returns the nearest time at which ordinary output should become idle.
    pub(super) fn next_activity_deadline(&self) -> Option<Instant> {
        self.panes
            .values()
            .filter_map(|pane| pane.activity.deadline())
            .min()
    }

    /// Returns panes with expired ordinary-output deadlines to idle. Returns
    /// whether any activity changed, allowing the runtime to avoid idle redraws.
    pub(super) fn expire_quiet_activity(&mut self, now: Instant) -> bool {
        let expired = self
            .panes
            .iter_mut()
            .filter_map(|(pane, target)| {
                target
                    .activity
                    .expire(now)
                    .map(|activity| (*pane, activity))
            })
            .collect::<Vec<_>>();
        for (pane, activity) in &expired {
            self.workspace.set_activity(*pane, *activity);
        }
        !expired.is_empty()
    }

    /// Raises a notification for `pane`: always an in-app status-bar notice, and
    /// a desktop notification too, throttling bare bells so a burst is one alert.
    fn raise_notification(
        &mut self,
        pane: PaneId,
        identifier: Option<NotificationId>,
        title: Option<String>,
        body: Option<String>,
    ) {
        let is_bell = title.is_none() && body.is_none();
        let scope = {
            let Some(target) = self.panes.get_mut(&pane) else {
                return;
            };
            if is_bell {
                let now = Instant::now();
                if target
                    .last_bell
                    .is_some_and(|last| now.duration_since(last) < BELL_THROTTLE)
                {
                    return;
                }
                target.last_bell = Some(now);
            }
            target.notification_scope
        };
        let Some(process) = self.workspace.process(pane) else {
            return;
        };
        let name = process.name().clone();
        let desktop_body = if is_bell {
            Some(AWAITING_INPUT_NOTICE.to_string())
        } else {
            body.clone()
        };
        // Prefer the body, fall back to the title (a title-only notification
        // still carries its text), then to a generic message for a bare bell.
        let message = body
            .or_else(|| title.clone())
            .unwrap_or_else(|| AWAITING_INPUT_NOTICE.to_string());
        self.notice = Some(format!("{}: {message}", name.as_ref()));
        let notification = Notification::builder()
            .pane(pane)
            .scope(scope)
            .source(name)
            .title(title)
            .body(desktop_body)
            .identifier(identifier)
            .build();
        if self.desktop_notifications_enabled()
            && let Some(notifier) = &self.notifier
        {
            notifier.notify(&notification);
        }
    }

    /// Closes a prior identified desktop notification. This remains active when
    /// delivery is toggled off so an already-visible notification can be removed.
    fn close_notification(&self, pane: PaneId, identifier: &NotificationId) {
        if let Some(scope) = self
            .panes
            .get(&pane)
            .map(|target| target.notification_scope)
            && let Some(notifier) = &self.notifier
        {
            notifier.close(pane, scope, identifier);
        }
    }

    /// Respawns a pane whose restart backoff has elapsed, but only if its
    /// generation still matches: a manual restart, stop, or newer schedule bumps
    /// the generation and so cancels this now-stale respawn.
    pub fn handle_respawn(&mut self, pane: PaneId, generation: SpawnGeneration) {
        if self.generations.get(&pane) != Some(&generation) {
            return;
        }
        if let Some((command, cwd)) = self.command_of(pane) {
            self.spawn(pane, command, cwd);
        }
    }

    /// Forcibly stops a command whose graceful manual-stop deadline elapsed,
    /// provided no exit, restart, or newer spawn superseded that request.
    pub fn handle_force_stop(&mut self, pane: PaneId, generation: SpawnGeneration) {
        if self.generations.get(&pane) != Some(&generation) {
            return;
        }
        let Some(target) = self.panes.get_mut(&pane) else {
            return;
        };
        if target.exit_intent.awaits_force_stop()
            && let Some(handle) = target.handle.as_mut()
            && handle.kill().is_err()
        {
            target.exit_intent = target.exit_intent.stop_delivery_failed();
        }
    }

    /// Draws the whole UI: sidebar, focused terminal, and status bar.
    pub fn render(&self, frame: &mut Frame) {
        let (sidebar_area, main_area, status_area) = areas(frame.area());
        let sidebar_focused = self.focus == Focus::Sidebar;
        let active_label = self.active_project_label();
        // Append the folder to a project's label when another project shares its
        // name, so duplicates in the tree are distinguishable.
        let other_projects: Vec<String> =
            self.other_projects()
                .into_iter()
                .filter_map(|index| self.projects.get(index).map(|project| (index, project)))
                .map(|(index, project)| {
                    let name = project.name().as_ref();
                    let duplicated = self.projects.iter().enumerate().any(|(other, candidate)| {
                        other != index && candidate.name().as_ref() == name
                    });
                    if duplicated {
                        format!("{name}  {}", label_from_config(project.config()))
                    } else {
                        name.to_string()
                    }
                })
                .collect();
        let selection = match self.project_cursor {
            Some(cursor) => sidebar::SidebarSelection::Project(cursor),
            None => sidebar::SidebarSelection::Process(*self.workspace.selected_index()),
        };
        sidebar::render(
            frame,
            sidebar_area,
            &self.workspace,
            sidebar_focused,
            &active_label,
            &other_projects,
            selection,
        );
        let (title, screen) = self.focused_view();
        terminal_pane::render(frame, main_area, &title, screen, !sidebar_focused);
        if self.workspace.processes().is_empty() {
            empty_state::render(frame, main_area);
        }
        let crashed = self
            .workspace
            .processes()
            .iter()
            .filter(|process| *process.state() == ProcessState::Crashed)
            .count();
        status_bar::render(
            frame,
            status_area,
            self.status_context(),
            crashed,
            self.notice.as_deref(),
            self.focus == Focus::Leader,
        );
        if let Some(overlay) = &self.overlay {
            overlay.render(frame);
        }
    }

    /// The slim hint set the status bar advertises for the current focus and
    /// sidebar selection. The full keymap lives in the `?` overlay.
    fn status_context(&self) -> StatusContext {
        if matches!(self.focus, Focus::Terminal | Focus::Leader) {
            return StatusContext::Terminal;
        }
        match self.project_cursor {
            Some(_) => StatusContext::Project,
            None if self.workspace.is_empty() => StatusContext::Empty,
            None => StatusContext::Process,
        }
    }

    /// The (title, screen) of the currently focused pane. A finished pane still
    /// has its parser, so its last screen keeps rendering.
    fn focused_view(&self) -> (String, Option<&Screen>) {
        match self.workspace.selected_process() {
            Some(process) => {
                let screen = self
                    .panes
                    .get(process.id())
                    .map(|pane| pane.parser.screen());
                (process.name().as_ref().to_string(), screen)
            },
            None => (APP_NAME.to_string(), None),
        }
    }

    /// Collects the processes to spawn as owned data, decoupled from `self`.
    fn spawn_list(&self) -> Vec<(PaneId, Option<CommandLine>, Option<PathBuf>)> {
        self.workspace
            .processes()
            .iter()
            // Only auto-start processes marked to; commands default off (waiting
            // for an explicit `s`), agents and terminals default on.
            .filter(|process| *process.autostart())
            .map(|process| {
                (
                    *process.id(),
                    process.command().clone(),
                    process.working_dir().clone(),
                )
            })
            .collect()
    }

    /// Spawns one process, wiring its parser, handle, and output sink. Any prior
    /// pane for the id is replaced with a fresh session; an absent command
    /// launches the user's login shell.
    fn spawn(&mut self, pane: PaneId, command: Option<CommandLine>, cwd: Option<PathBuf>) {
        let generation = self.bump_generation(pane);
        let project = self
            .current_config
            .as_ref()
            .map(|config| path::normalize(config));
        let request = SpawnRequest::builder()
            .command(command)
            .working_dir(cwd)
            .project(project)
            .size(self.pane_size)
            .build();
        let sink = ChannelOutputSink::new(pane, generation, self.events.clone());
        match self.runner.spawn(request, Box::new(sink)) {
            Ok(handle) => {
                let notification_scope = self.allocate_notification_scope();
                let parser = Parser::new(
                    self.pane_size.rows().into_inner(),
                    self.pane_size.cols().into_inner(),
                    SCROLLBACK_LINES,
                );
                self.panes.insert(pane, Pane {
                    parser,
                    notification_scope,
                    signals: SignalReader::new(),
                    activity: ActivityTracker::default(),
                    last_bell: None,
                    handle: Some(handle),
                    started_at: Instant::now(),
                    exit_intent: ExitIntent::FollowPolicy,
                    config_membership: ConfigMembership::Tracked,
                });
                self.workspace.set_state(pane, ProcessState::Running);
                // A fresh child inherits none of the prior generation's activity.
                self.workspace.set_activity(pane, ActivityState::Idle);
            },
            Err(_) => {
                self.deactivate(pane);
                // A transient spawn failure must not abandon the restart policy:
                // stay in the backoff path for always/on_failure, else crash.
                if self.workspace.should_restart(pane, ExitOutcome::Failed) {
                    self.workspace.set_state(pane, ProcessState::Restarting);
                    self.schedule_restart(pane, Instant::now());
                } else {
                    self.workspace.set_state(pane, ProcessState::Crashed);
                }
            },
        }
    }

    /// Bumps and returns a pane's restart generation, invalidating any respawn
    /// scheduled against an older generation.
    fn bump_generation(&mut self, pane: PaneId) -> SpawnGeneration {
        let entry = self
            .generations
            .entry(pane)
            .or_insert_with(SpawnGeneration::initial);
        *entry = entry.next();
        *entry
    }

    /// Allocates an identity that is never reused by another terminal lifetime
    /// during this application run.
    fn allocate_notification_scope(&mut self) -> NotificationScope {
        let scope = self.next_notification_scope;
        self.next_notification_scope = scope.next();
        scope
    }

    /// Drops a pane's live handle but keeps its parser, so the final screen and
    /// scrollback remain visible after the process exits.
    fn deactivate(&mut self, pane: PaneId) {
        if let Some(target) = self.panes.get_mut(&pane) {
            target.activity.reset();
            target.handle = None;
            target.exit_intent = ExitIntent::FollowPolicy;
        }
    }

    /// Drops all trace of `pane`: its terminal, generation, restart bookkeeping,
    /// and workspace entry. Used to retire a process removed from the config.
    fn retire_pane(&mut self, pane: PaneId) {
        self.panes.remove(&pane);
        self.generations.remove(&pane);
        self.restart_attempts.remove(&pane);
        self.workspace.remove(pane);
    }

    /// Handles a process exit: honor a stop, force-restart, back-off restart per
    /// policy, or record the resting outcome.
    fn handle_exit(&mut self, pane: PaneId, outcome: ExitOutcome) {
        let Some((exit_intent, config_membership, started_at)) = self
            .panes
            .get(&pane)
            .map(|p| (p.exit_intent, p.config_membership, p.started_at))
        else {
            return;
        };

        // A process removed from the config while running is retired now that it
        // has exited, rather than restarting a process that no longer exists.
        if config_membership == ConfigMembership::RetireOnExit {
            self.retire_pane(pane);
            self.advance_pending_switch(pane);
            return;
        }

        // The child is gone; drop any attention state so a restart backoff or a
        // crashed/exited pane never keeps showing the waiting-for-user marker.
        self.workspace.set_activity(pane, ActivityState::Idle);
        match exit_intent {
            ExitIntent::StopRetryable | ExitIntent::StopInFlight => {
                self.deactivate(pane);
                self.workspace.set_state(pane, ProcessState::Exited);
            },
            ExitIntent::Restart => {
                self.restart_attempts.remove(&pane);
                self.workspace.set_state(pane, ProcessState::Restarting);
                if let Some((command, cwd)) = self.command_of(pane) {
                    self.spawn(pane, command, cwd);
                }
            },
            ExitIntent::FollowPolicy if self.workspace.should_restart(pane, outcome) => {
                self.workspace.set_state(pane, ProcessState::Restarting);
                self.deactivate(pane);
                self.schedule_restart(pane, started_at);
            },
            ExitIntent::FollowPolicy => {
                self.deactivate(pane);
                self.workspace.set_state(pane, exit_state(outcome));
            },
        }
        self.advance_pending_switch(pane);
    }

    /// Schedules an automatic restart after a backoff, growing the delay for
    /// repeated fast failures and resetting it after a stable run. The captured
    /// generation lets a later manual action cancel this respawn.
    fn schedule_restart(&mut self, pane: PaneId, started_at: Instant) {
        let attempts = if started_at.elapsed() >= RESTART_STABLE_RUN {
            0
        } else {
            self.restart_attempts.get(&pane).copied().unwrap_or(0)
        };
        let delay = (RESTART_BACKOFF_BASE * 2u32.pow(attempts.min(RESTART_BACKOFF_MAX_EXP)))
            .min(RESTART_BACKOFF_MAX);
        self.restart_attempts.insert(pane, attempts + 1);
        let generation = self.bump_generation(pane);
        let sender = self.events.clone();
        thread::spawn(move || {
            thread::sleep(delay);
            let _ = sender.send(RuntimeEvent::Respawn { pane, generation });
        });
    }

    /// Schedules hard-kill escalation for a manually stopped command. The
    /// generation and exit intent make the event harmless after exit/restart.
    fn schedule_force_stop(&self, pane: PaneId, generation: SpawnGeneration) {
        let sender = self.events.clone();
        thread::spawn(move || {
            thread::sleep(COMMAND_STOP_GRACE);
            let _ = sender.send(RuntimeEvent::ForceStop { pane, generation });
        });
    }

    /// The launch command and cwd of the process owning `pane`.
    fn command_of(&self, pane: PaneId) -> Option<(Option<CommandLine>, Option<PathBuf>)> {
        self.workspace
            .process(pane)
            .map(|process| (process.command().clone(), process.working_dir().clone()))
    }

    /// Handles a key: leader chord, command, or forward to the focused pane.
    fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        // A key press dismisses any transient notice from the previous action.
        self.notice = None;
        match &self.overlay {
            // Help is a read-only reference; any key closes it.
            Some(Overlay::Help) => {
                self.overlay = None;
                return;
            },
            Some(Overlay::ConfirmOverwrite { .. } | Overlay::ConfirmRemoval { .. }) => {
                self.handle_confirm_key(key);
                return;
            },
            Some(Overlay::Form(_)) => {
                self.handle_form_key(key);
                return;
            },
            Some(Overlay::Switcher(_)) => {
                self.handle_switcher_key(key);
                return;
            },
            None => {},
        }
        match self.focus {
            Focus::Sidebar => self.handle_sidebar_key(key),
            Focus::Terminal if is_leader(key) => self.focus = Focus::Leader,
            Focus::Terminal => self.forward_key(key),
            Focus::Leader => {
                self.focus = Focus::Terminal;
                self.handle_leader_command(key);
            },
        }
    }

    /// Handles a key while the sidebar is focused: direct navigation and actions.
    fn handle_sidebar_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.running = false,
            KeyCode::Char('j') | KeyCode::Down => self.sidebar_down(),
            KeyCode::Char('k') | KeyCode::Up => self.sidebar_up(),
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => self.sidebar_select(),
            KeyCode::Char('h') | KeyCode::Left => self.project_cursor = None,
            // On a project row: `d` removes the project; on a process row the
            // process actions apply. Neither leaks across contexts.
            KeyCode::Char('d') if self.project_cursor.is_some() => {
                self.confirm_remove_selected_project();
            },
            KeyCode::Char('t') if self.project_cursor.is_none() => self.toggle_selected_autostart(),
            KeyCode::Char('s') if self.project_cursor.is_none() => self.toggle_selected(),
            KeyCode::Char('r') if self.project_cursor.is_none() => self.restart_selected(),
            KeyCode::Char('p') if self.project_cursor.is_none() => self.toggle_pause_selected(),
            KeyCode::Char('x') if self.project_cursor.is_none() => self.stop_selected(),
            KeyCode::Char('a') => self.open_add_process_form(),
            KeyCode::Char('n') => self.open_new_project_form(),
            KeyCode::Char('N') => self.toggle_desktop_notifications(),
            KeyCode::Char('o') => self.open_switcher(),
            KeyCode::Char('?') => self.overlay = Some(Overlay::Help),
            _ => {},
        }
    }

    /// Indices into `self.projects` of every project that is not the active one,
    /// in registry order - the collapsed rows below the active project.
    fn other_projects(&self) -> Vec<usize> {
        let active = self.current_project_index(&self.projects);
        (0..self.projects.len())
            .filter(|index| Some(*index) != active)
            .collect()
    }

    /// Moves the sidebar selection down: through the active project's processes,
    /// then onto the collapsed project rows, wrapping back to the top.
    fn sidebar_down(&mut self) {
        let processes = self.workspace.processes().len();
        let others = self.other_projects().len();
        match self.project_cursor {
            None => {
                let index = *self.workspace.selected_index();
                if index + 1 < processes {
                    self.workspace.select_at(index + 1);
                } else if others > 0 {
                    self.project_cursor = Some(0);
                } else if processes > 0 {
                    self.workspace.select_at(0);
                }
            },
            Some(cursor) if cursor + 1 < others => self.project_cursor = Some(cursor + 1),
            Some(_) => {
                self.project_cursor = None;
                self.workspace.select_at(0);
            },
        }
    }

    /// Moves the sidebar selection up, mirroring [`Self::sidebar_down`].
    fn sidebar_up(&mut self) {
        let processes = self.workspace.processes().len();
        let others = self.other_projects().len();
        match self.project_cursor {
            None => {
                let index = *self.workspace.selected_index();
                if index > 0 {
                    self.workspace.select_at(index - 1);
                } else if others > 0 {
                    self.project_cursor = Some(others - 1);
                }
            },
            Some(0) => {
                self.project_cursor = None;
                if processes > 0 {
                    self.workspace.select_at(processes - 1);
                }
            },
            Some(cursor) => self.project_cursor = Some(cursor - 1),
        }
    }

    /// Acts on the sidebar selection: attach to the selected process, or switch
    /// into the selected project.
    fn sidebar_select(&mut self) {
        match self.project_cursor {
            Some(cursor) => self.activate_other_project(cursor),
            None => self.focus = Focus::Terminal,
        }
    }

    /// Switches to the `cursor`-th collapsed project, making it active.
    fn activate_other_project(&mut self, cursor: usize) {
        let Some(index) = self.other_projects().get(cursor).copied() else {
            return;
        };
        let Some(project) = self.projects.get(index).cloned() else {
            return;
        };
        match self.registry.workspace(project.config()) {
            Ok(config) => {
                self.project_cursor = None;
                self.begin_switch(config, project.config().clone());
            },
            Err(err) => self.report_project_open_failure(&project, &err),
        }
    }

    /// Handles a project whose config could not be opened: if the file is gone,
    /// offer to remove the stale entry right away; if it is present but
    /// unreadable, just report it (removing an entry whose file still exists
    /// would be destructive).
    fn report_project_open_failure(&mut self, project: &Project, err: &ConfigError) {
        if self.registry.workspace_exists(project.config()) {
            self.notice = Some(format!("{}: {err}", project.name().as_ref()));
            return;
        }
        let message = format!("{}'s config file is missing.", project.name().as_ref());
        self.confirm_remove_project(project, message);
    }

    /// Opens a confirmation to remove `project` from the registry, closing any
    /// project overlay first. Shared by activation failures and the sidebar `d`.
    fn confirm_remove_project(&mut self, project: &Project, message: String) {
        self.project_cursor = None;
        self.overlay = Some(Overlay::ConfirmRemoval {
            message,
            config_path: project.config().clone(),
        });
    }

    /// Confirms removal of the project on the selected sidebar row.
    fn confirm_remove_selected_project(&mut self) {
        let Some(cursor) = self.project_cursor else {
            return;
        };
        let Some(project) = self
            .other_projects()
            .get(cursor)
            .and_then(|index| self.projects.get(*index))
            .cloned()
        else {
            return;
        };
        // The synthetic launched-project row has no registry entry: "removing" it
        // would save an unchanged list and immediately reappear, so refuse.
        if self.is_synthetic_launched(&project) {
            self.notice = Some(CANNOT_REMOVE_LAUNCHED.to_string());
            return;
        }
        let message = format!("Remove project '{}'?", project.name().as_ref());
        self.confirm_remove_project(&project, message);
    }

    /// Whether `project` is the unsaved launched-project row synthesized for the
    /// tree rather than a registered project.
    fn is_synthetic_launched(&self, project: &Project) -> bool {
        self.launched_project_membership == LaunchedProjectMembership::Synthetic
            && path::normalize(project.config()) == path::normalize(&self.launched_config)
    }

    /// Flips the selected process's autostart on or off. The explicit value is
    /// written to the matching spec first, and the live process is updated only
    /// when that write both succeeds and actually found a spec to change, so the
    /// sidebar never shows a state the config did not record. The spec is located
    /// by the process's full resolved identity, and among identical rows by the
    /// selected one's position within them, so the persisted change lands on the
    /// row the user picked whatever order a reconcile left the rows in.
    fn toggle_selected_autostart(&mut self) {
        let Some(config_path) = self.current_config.clone() else {
            return;
        };
        let Some(process) = self.workspace.selected_process() else {
            return;
        };
        let pane = *process.id();
        let autostart = !*process.autostart();
        let target = SpecMatch::of(process);
        let occurrence = self
            .workspace
            .processes()
            .iter()
            .filter(|candidate| target.matches_process(candidate))
            .position(|candidate| *candidate.id() == pane)
            .unwrap_or(0);

        let mut edited = false;
        let mut apply = |config: WorkspaceConfig| {
            let (config, found) = target.with_autostart(config, occurrence, Some(autostart));
            edited = found;
            config
        };
        match self.registry.update_workspace(&config_path, &mut apply) {
            Ok(()) if edited => self.workspace.set_autostart(pane, autostart),
            Ok(()) => self.notice = Some(AUTOSTART_UNTRACKED.to_string()),
            Err(_) => self.notice = Some(WORKSPACE_SAVE_ERROR.to_string()),
        }
    }

    /// Removes the registered project at `config_path` from the registry. Reads
    /// the persisted list so the in-memory launched-project entry is never saved.
    fn remove_project(&mut self, config_path: &Path) {
        let Ok(mut projects) = self.registry.projects() else {
            self.notice = Some(REGISTRY_SAVE_ERROR.to_string());
            return;
        };
        let target = path::normalize(config_path);
        projects.retain(|project| path::normalize(project.config()) != target);
        if self.registry.save(&projects).is_ok() {
            self.project_cursor = None;
            self.refresh_projects();
        } else {
            self.notice = Some(REGISTRY_SAVE_ERROR.to_string());
        }
    }

    /// The active project's display name: its registered name, else the config's
    /// parent directory, else the app name - so its header is never blank.
    fn active_project_label(&self) -> String {
        if let Some(index) = self.current_project_index(&self.projects)
            && let Some(project) = self.projects.get(index)
        {
            return project.name().as_ref().to_string();
        }
        self.current_config
            .as_deref()
            .map(label_from_config)
            .unwrap_or_else(|| APP_NAME.to_string())
    }

    /// Handles a command key pressed after the leader while a terminal is focused.
    fn handle_leader_command(&mut self, key: KeyEvent) {
        if is_leader(key) {
            self.forward_key(key);
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.running = false,
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Esc => self.focus = Focus::Sidebar,
            KeyCode::Char('j') | KeyCode::Down => self.workspace.select_next(),
            KeyCode::Char('k') | KeyCode::Up => self.workspace.select_previous(),
            KeyCode::Char('s') => self.toggle_selected(),
            KeyCode::Char('r') => self.restart_selected(),
            KeyCode::Char('p') => self.toggle_pause_selected(),
            KeyCode::Char('a') => self.open_add_process_form(),
            KeyCode::Char('n') => self.open_new_project_form(),
            KeyCode::Char('N') => self.toggle_desktop_notifications(),
            KeyCode::Char('o') => self.open_switcher(),
            KeyCode::Char('x') => self.stop_selected(),
            KeyCode::Char('?') => self.overlay = Some(Overlay::Help),
            _ => {},
        }
    }

    /// Toggles the selected process: stop it if alive, start it if not.
    fn toggle_selected(&mut self) {
        let Some(pane) = self.selected_pane() else {
            return;
        };
        if self.panes.get(&pane).is_some_and(|p| p.handle.is_some()) {
            self.stop_selected();
        } else {
            self.start_selected();
        }
    }

    /// Starts the selected process if it is not currently running.
    fn start_selected(&mut self) {
        let Some(pane) = self.selected_pane() else {
            return;
        };
        if let Some((command, cwd)) = self.command_of(pane) {
            self.spawn(pane, command, cwd);
        }
    }

    /// Toggles the selected process between paused and running via SIGSTOP and
    /// SIGCONT. Ignores absent children and panes already stopping or restarting.
    fn toggle_pause_selected(&mut self) {
        let Some(pane) = self.selected_pane() else {
            return;
        };
        let is_paused = self
            .workspace
            .process(pane)
            .is_some_and(|process| *process.state() == ProcessState::Paused);
        let now_paused = {
            let Some(target) = self.panes.get_mut(&pane) else {
                return;
            };
            if target.exit_intent != ExitIntent::FollowPolicy {
                return;
            }
            let Some(handle) = target.handle.as_mut() else {
                return;
            };
            let next = !is_paused;
            let signalled = if next {
                handle.pause()
            } else {
                handle.resume()
            };
            // A failed signal leaves the child unchanged: do not advertise a
            // transition that did not happen.
            if signalled.is_err() {
                return;
            }
            next
        };
        let state = if now_paused {
            ProcessState::Paused
        } else {
            ProcessState::Running
        };
        self.workspace.set_state(pane, state);
    }

    /// Opens the project switcher, reloading the registry so on-disk edits are
    /// picked up. Highlights the current project, else the first one.
    fn open_switcher(&mut self) {
        let (projects, error) = match self.registry.projects() {
            Ok(projects) => (projects, None),
            Err(err) => (Vec::new(), Some(err.to_string())),
        };
        let current = self.current_project_index(&projects);
        let selected = current.unwrap_or(0);
        let preview = Switcher::preview(self.registry.as_ref(), projects.get(selected));
        self.overlay = Some(Overlay::Switcher(Switcher {
            projects,
            selected,
            current,
            error,
            preview,
        }));
    }

    /// Opens `form` with `intent`, retaining an open switcher for cancellation.
    fn open_form(&mut self, form: Form, intent: FormIntent) {
        let switcher = match self.overlay.take() {
            Some(Overlay::Switcher(switcher)) => Some(switcher),
            _ => None,
        };
        self.overlay = Some(Overlay::Form(FormOverlay {
            modal: FormModal {
                form,
                intent,
                error: None,
            },
            switcher,
        }));
    }

    /// Opens the save-current-project form (one name field).
    fn open_save_project_form(&mut self) {
        let form = Form::new(SAVE_PROJECT_TITLE, vec![Field::text(NAME_FIELD)]);
        self.open_form(form, FormIntent::SaveCurrentProject);
    }

    /// Opens the new-project form: a name, and a folder prefilled with the
    /// current directory so the common case is just typing a name.
    fn open_new_project_form(&mut self) {
        let folder = std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let form = Form::new(NEW_PROJECT_TITLE, vec![
            Field::text(NAME_FIELD),
            Field::path(FOLDER_FIELD, &folder),
        ]);
        self.open_form(form, FormIntent::NewProject);
    }

    /// Opens the add-process form: a kind choice, a name, and a command.
    fn open_add_process_form(&mut self) {
        let form = Form::new(ADD_PROCESS_TITLE, vec![
            Field::choice(KIND_FIELD, &KIND_OPTIONS),
            Field::text(NAME_FIELD),
            Field::text(COMMAND_FIELD),
        ]);
        self.open_form(form, FormIntent::AddProcess);
    }

    /// Removes the highlighted project from the registry.
    fn remove_selected_project(&mut self) {
        let Some(switcher) = self.switcher() else {
            return;
        };
        let selected = switcher.selected;
        if selected >= switcher.projects.len() {
            return;
        }
        let mut projects = switcher.projects.clone();
        projects.remove(selected);
        if self.registry.save(&projects).is_ok() {
            self.refresh_projects();
            self.refresh_switcher();
        } else if let Some(switcher) = self.switcher_mut() {
            switcher.error = Some(REGISTRY_SAVE_ERROR.to_string());
        }
    }

    /// Reloads the open switcher from the registry after a change.
    fn refresh_switcher(&mut self) {
        if self.switcher().is_none() {
            return;
        }
        let (projects, error) = match self.registry.projects() {
            Ok(projects) => (projects, None),
            Err(err) => (Vec::new(), Some(err.to_string())),
        };
        let current = self.current_project_index(&projects);
        let selected = current.unwrap_or(0).min(projects.len().saturating_sub(1));
        let preview = Switcher::preview(self.registry.as_ref(), projects.get(selected));
        self.overlay = Some(Overlay::Switcher(Switcher {
            projects,
            selected,
            current,
            error,
            preview,
        }));
    }

    /// Recomputes the open switcher's cached preview for its current selection,
    /// after the highlight moves.
    fn update_switcher_preview(&mut self) {
        let Some(project) = self
            .switcher()
            .and_then(|switcher| switcher.projects.get(switcher.selected).cloned())
        else {
            return;
        };
        let preview = Switcher::preview(self.registry.as_ref(), Some(&project));
        if let Some(switcher) = self.switcher_mut() {
            switcher.preview = preview;
        }
    }

    /// Handles a key while a form is open: edit, submit, or cancel.
    fn handle_form_key(&mut self, key: KeyEvent) {
        let Some(modal) = self.form_mut() else {
            return;
        };
        let before = (modal.form.active(), modal.form.active_path_value());
        let outcome = modal.form.handle(key);
        // Recompute completions only when the active field or its value changed,
        // so dropdown navigation (which only moves the highlight) keeps it.
        let changed = (modal.form.active(), modal.form.active_path_value()) != before;
        match outcome {
            FormOutcome::Continue => {
                if changed {
                    self.refresh_completions();
                }
            },
            // Acceptance closes the dropdown deliberately; leave it closed so the
            // next Enter submits instead of accepting a child of the chosen dir.
            FormOutcome::Accepted => {},
            FormOutcome::Cancel => self.close_overlay(),
            FormOutcome::Submit => self.submit_form(),
        }
    }

    /// Recomputes the active path field's autocomplete candidates. With a worker
    /// wired, it dispatches a generation-tagged request and clears the field so
    /// no stale suggestion is shown until the matching reply arrives; otherwise
    /// it completes inline.
    fn refresh_completions(&mut self) {
        let Some(partial) = self.form().and_then(|modal| modal.form.active_path_value()) else {
            return;
        };
        // The candidates to show right now: none while an async request is in
        // flight (its reply repopulates them, and until then no navigation or
        // acceptance can act on suggestions that no longer match the edited
        // value), or the inline result otherwise.
        let candidates = match &mut self.completion_mode {
            CompletionMode::Worker {
                requests,
                generation,
            } => {
                *generation = generation.next();
                let _ = requests.send(CompletionRequest {
                    generation: *generation,
                    partial,
                });
                Vec::new()
            },
            CompletionMode::Inline(completer) => completer.complete_dir(&partial),
        };
        if let Some(modal) = self.form_mut() {
            modal.form.set_active_candidates(candidates);
        }
    }

    /// Applies worker-computed completions, ignoring any that a later edit has
    /// already superseded.
    pub fn handle_completions(
        &mut self,
        generation: CompletionGeneration,
        candidates: Vec<String>,
    ) {
        let CompletionMode::Worker {
            generation: current,
            ..
        } = &self.completion_mode
        else {
            return;
        };
        if generation != *current {
            return;
        }
        if let Some(modal) = self.form_mut() {
            modal.form.set_active_candidates(candidates);
        }
    }

    /// Executes the open form's intent with its collected values.
    fn submit_form(&mut self) {
        let Some(modal) = self.form() else {
            return;
        };
        let values = modal.form.values();
        let intent = modal.intent;
        match intent {
            FormIntent::SaveCurrentProject => self.save_current_project(&values),
            FormIntent::NewProject => self.new_project(&values),
            FormIntent::AddProcess => self.add_process(&values),
        }
    }

    /// Registers the current workspace under the typed name. A blank or invalid
    /// name leaves the form open.
    fn save_current_project(&mut self, values: &[String]) {
        let (Some(name), Some(config)) = (values.first(), self.current_config.clone()) else {
            return;
        };
        let Ok(name) = ProjectName::try_new(name.trim()) else {
            return;
        };
        if self.try_register(Project::builder().name(name).config(config).build()) {
            self.close_overlay();
            self.refresh_projects();
            self.refresh_switcher();
        }
    }

    /// Creates a new project, asking first if the folder already holds a config.
    /// A blank or invalid field leaves the form open.
    fn new_project(&mut self, values: &[String]) {
        let (Some(name), Some(folder)) = (values.first(), values.get(1)) else {
            return;
        };
        let Ok(name) = ProjectName::try_new(name.trim()) else {
            return;
        };
        let folder = folder.trim();
        if folder.is_empty() {
            return;
        }
        let config_path = PathBuf::from(folder).join(PROJECT_CONFIG_FILE);
        if self.registry.workspace_exists(&config_path) {
            self.confirm_overwrite(name, config_path);
            return;
        }
        self.create_project(name, config_path);
    }

    /// Opens a confirmation over the form before overwriting an existing config.
    /// The form is kept so a failed overwrite can be retried without refilling.
    fn confirm_overwrite(&mut self, name: ProjectName, config_path: PathBuf) {
        let Some(Overlay::Form(form)) = self.overlay.take() else {
            return;
        };
        self.overlay = Some(Overlay::ConfirmOverwrite {
            form,
            name,
            config_path,
        });
    }

    /// Registers the project, then writes its starter config. Registration comes
    /// first so a failed write leaves a recoverable dangling entry (a retry heals
    /// it) rather than a stranded file that would block re-creation.
    fn create_project(&mut self, name: ProjectName, config_path: PathBuf) {
        let project = Project::builder()
            .name(name)
            .config(config_path.clone())
            .build();
        if !self.try_register(project) {
            return;
        }
        if self
            .registry
            .save_workspace(&config_path, &starter_config())
            .is_err()
        {
            self.report_error(WORKSPACE_SAVE_ERROR);
            return;
        }
        self.close_overlay();
        self.refresh_projects();
        self.refresh_switcher();
    }

    /// Handles a key while a confirmation is open: accept (y / Enter) or cancel.
    fn handle_confirm_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y') => {
                let overlay = self.overlay.take();
                match overlay {
                    Some(Overlay::ConfirmOverwrite {
                        form,
                        name,
                        config_path,
                    }) => {
                        self.overlay = Some(Overlay::Form(form));
                        self.create_project(name, config_path);
                    },
                    Some(Overlay::ConfirmRemoval { config_path, .. }) => {
                        self.remove_project(&config_path);
                    },
                    other => self.overlay = other,
                }
            },
            KeyCode::Esc | KeyCode::Char('n') => {
                self.close_overlay();
            },
            _ => {},
        }
    }

    /// Reports an error on the active form or project switcher.
    fn report_error(&mut self, message: &str) {
        if let Some(modal) = self.form_mut() {
            modal.error = Some(message.to_string());
        } else if let Some(switcher) = self.switcher_mut() {
            switcher.error = Some(message.to_string());
        }
    }

    /// Adds `project` to the registry, replacing any entry for the same config
    /// path. Returns whether it was persisted, setting a form error on failure;
    /// leaves the form open so the caller decides when to close it.
    fn try_register(&mut self, project: Project) -> bool {
        let mut projects = match self.registry.projects() {
            Ok(projects) => projects,
            Err(err) => {
                self.report_error(&err.to_string());
                return false;
            },
        };
        let target = path::normalize(project.config());
        projects.retain(|existing| path::normalize(existing.config()) != target);
        projects.push(project);
        if self.registry.save(&projects).is_ok() {
            true
        } else {
            self.report_error(REGISTRY_SAVE_ERROR);
            false
        }
    }

    /// Adds a process to the current workspace: appends it to the config file
    /// and reloads the project so it starts. A blank or invalid field leaves the
    /// form open.
    fn add_process(&mut self, values: &[String]) {
        let (Some(kind), Some(name), Some(command)) =
            (values.first(), values.get(1), values.get(2))
        else {
            return;
        };
        let Some(config_path) = self.current_config.clone() else {
            return;
        };
        let kind = match kind.as_str() {
            KIND_AGENT => ProcessKind::Agent,
            KIND_TERMINAL => ProcessKind::Terminal,
            KIND_COMMAND => ProcessKind::Command,
            _ => return,
        };
        let Ok(name) = ProcessName::try_new(name.trim()) else {
            return;
        };
        let command = command.trim();
        let command = if command.is_empty() {
            None
        } else {
            match CommandLine::try_new(command) {
                Ok(command) => Some(command),
                Err(_) => return,
            }
        };
        let spec = ProcessSpec::builder().name(name).command(command).build();
        // Route through the registry's locked read-modify-write, the same one
        // `muster run` uses, so an overlapping CLI add and this add cannot
        // silently discard each other.
        let mut append = |config: WorkspaceConfig| match kind {
            ProcessKind::Agent => {
                let mut specs = config.agents().clone();
                specs.push(spec.clone());
                config.with_agents(specs)
            },
            ProcessKind::Terminal => {
                let mut specs = config.terminals().clone();
                specs.push(spec.clone());
                config.with_terminals(specs)
            },
            ProcessKind::Command => {
                let mut specs = config.commands().clone();
                specs.push(spec.clone());
                config.with_commands(specs)
            },
        };
        if self
            .registry
            .update_workspace(&config_path, &mut append)
            .is_err()
        {
            self.report_error(WORKSPACE_SAVE_ERROR);
            return;
        }
        // Reload the now-persisted config (which also picks up any concurrent
        // additions) to drive the switch.
        let config = match self.registry.workspace(&config_path) {
            Ok(config) => config,
            Err(err) => {
                self.report_error(&err.to_string());
                return;
            },
        };
        self.overlay = None;
        self.begin_switch(config, config_path);
    }

    /// Handles a key while the switcher is open: navigate, jump by number,
    /// confirm the highlighted project, or cancel.
    fn handle_switcher_key(&mut self, key: KeyEvent) {
        let Some(switcher) = self.switcher() else {
            return;
        };
        let count = switcher.projects.len();
        let selected = switcher.selected;
        match key.code {
            KeyCode::Esc => self.overlay = None,
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(switcher) = self.switcher_mut()
                    && count > 0
                {
                    switcher.selected = (selected + 1) % count;
                }
                self.update_switcher_preview();
            },
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(switcher) = self.switcher_mut()
                    && count > 0
                {
                    switcher.selected = if selected == 0 {
                        count - 1
                    } else {
                        selected - 1
                    };
                }
                self.update_switcher_preview();
            },
            KeyCode::Enter => {
                if count > 0 {
                    self.switch_to(selected);
                }
            },
            KeyCode::Char('n') => self.open_new_project_form(),
            KeyCode::Char('s') => self.open_save_project_form(),
            KeyCode::Char('a') => self.open_add_process_form(),
            KeyCode::Char('d') => self.remove_selected_project(),
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let index = usize::from(c as u8 - b'1');
                if index < count {
                    self.switch_to(index);
                }
            },
            _ => {},
        }
    }

    /// Switches to the project at `index`: on success loads its workspace in
    /// place, otherwise surfaces the failure and keeps the overlay open.
    fn switch_to(&mut self, index: usize) {
        let Some(project) = self
            .switcher()
            .and_then(|switcher| switcher.projects.get(index))
            .cloned()
        else {
            return;
        };
        match self.registry.workspace(project.config()) {
            Ok(config) => {
                self.begin_switch(config, project.config().clone());
                self.overlay = None;
            },
            Err(err) if self.registry.workspace_exists(project.config()) => {
                if let Some(switcher) = self.switcher_mut() {
                    switcher.selected = index;
                    switcher.error = Some(err.to_string());
                }
            },
            // The file is gone: close the switcher and offer to remove the stale
            // entry, the same one-step flow as from the sidebar tree.
            Err(err) => self.report_project_open_failure(&project, &err),
        }
    }

    /// Begins switching to `config`: kills the current children and defers the
    /// load until they all report exit, so the replacement processes never race
    /// the old ones for ports or other resources. Loads at once when nothing is
    /// running. The children receive stop intent so their exits do not restart.
    fn begin_switch(&mut self, config: WorkspaceConfig, config_path: PathBuf) {
        let live: HashSet<PaneId> = self
            .panes
            .iter()
            .filter(|(_, pane)| pane.handle.is_some())
            .map(|(id, _)| *id)
            .collect();
        if live.is_empty() {
            self.load_project(config, config_path);
            return;
        }
        for pane in &live {
            if let Some(target) = self.panes.get_mut(pane)
                && let Some(handle) = target.handle.as_mut()
            {
                target.exit_intent = target.exit_intent.request_stop();
                if handle.kill().is_ok() {
                    target.exit_intent = target.exit_intent.stop_delivered();
                }
            }
        }
        self.pending_switch = Some(PendingSwitch {
            config,
            config_path,
            waiting: live,
        });
    }

    /// Removes `pane` from a deferred switch's wait set and, once every old child
    /// has exited, loads the pending project.
    fn advance_pending_switch(&mut self, pane: PaneId) {
        let ready = match self.pending_switch.as_mut() {
            Some(pending) => {
                pending.waiting.remove(&pane);
                pending.waiting.is_empty()
            },
            None => return,
        };
        if ready && let Some(pending) = self.pending_switch.take() {
            // Reload from disk: an edit or `muster run --project ...` targeting
            // the destination during the wait happened before its watcher was
            // installed, so the snapshot read at begin_switch time may be stale.
            let config = self
                .registry
                .workspace(&pending.config_path)
                .unwrap_or(pending.config);
            self.load_project(config, pending.config_path);
        }
    }

    /// Tears down the current project and starts `config` in its place. The
    /// generation map is kept so late output from the killed children is
    /// discarded as stale once the new panes bump their generations.
    fn load_project(&mut self, config: WorkspaceConfig, config_path: PathBuf) {
        for pane in self.panes.values_mut() {
            if let Some(handle) = pane.handle.as_mut() {
                let _ = handle.kill();
            }
        }
        self.panes.clear();
        self.restart_attempts.clear();
        self.workspace = Workspace::builder()
            .processes(config.to_processes())
            .build();
        self.current_config = Some(config_path);
        self.focus = Focus::Sidebar;
        self.rewatch_config();
        self.start();
    }

    /// Index of the project whose config resolves to the one loaded now. Paths
    /// are normalized (`~` expanded, made absolute, canonicalized) so a relative
    /// CLI path matches a `~`-prefixed or absolute registry entry.
    fn current_project_index(&self, projects: &[Project]) -> Option<usize> {
        let current = path::normalize(self.current_config.as_deref()?);
        projects
            .iter()
            .position(|project| path::normalize(project.config()) == current)
    }

    /// Forwards an encoded key to the focused pane's PTY, if it is alive.
    fn forward_key(&mut self, key: KeyEvent) {
        let Some(bytes) = input::encode_key(key) else {
            return;
        };
        if let Some(pane) = self.selected_pane()
            && let Some(target) = self.panes.get_mut(&pane)
            && let Some(handle) = target.handle.as_mut()
        {
            let _ = handle.write_input(&bytes);
        }
    }

    /// Force-restarts the selected process immediately, even under a Never
    /// policy: kills a live process, or respawns a finished one at once.
    fn restart_selected(&mut self) {
        let Some(pane) = self.selected_pane() else {
            return;
        };
        let alive = self.panes.get(&pane).is_some_and(|p| p.handle.is_some());
        if alive {
            if let Some(target) = self.panes.get_mut(&pane) {
                // Supersede a still-pending stop so the newer restart intent wins
                // when the exit lands.
                target.exit_intent = target.exit_intent.request_restart();
                if let Some(handle) = target.handle.as_mut() {
                    let _ = handle.kill();
                }
            }
        } else if let Some((command, cwd)) = self.command_of(pane) {
            self.spawn(pane, command, cwd);
        }
    }

    /// Stops the selected process without respawning it. Repeated requests reuse
    /// a successfully delivered request, while failed delivery remains retryable.
    fn stop_selected(&mut self) {
        let Some(pane) = self.selected_pane() else {
            return;
        };
        if self.panes.get(&pane).is_some_and(|target| {
            target.handle.is_some() && !target.exit_intent.accepts_stop_request()
        }) {
            return;
        }
        let alive = self.panes.get(&pane).is_some_and(|p| p.handle.is_some());
        if alive {
            let graceful = self
                .workspace
                .process(pane)
                .is_some_and(|process| *process.kind() == ProcessKind::Command);
            let generation = self.generations.get(&pane).copied();
            let mut awaiting_grace = false;
            if let Some(target) = self.panes.get_mut(&pane) {
                target.exit_intent = target.exit_intent.request_stop();
                if let Some(handle) = target.handle.as_mut() {
                    let stop_requested = if graceful {
                        match handle.terminate(COMMAND_STOP_GRACE) {
                            Ok(()) => {
                                awaiting_grace = true;
                                true
                            },
                            Err(_) => handle.kill().is_ok(),
                        }
                    } else {
                        handle.kill().is_ok()
                    };
                    if stop_requested {
                        target.exit_intent = target.exit_intent.stop_delivered();
                    }
                }
            }
            if awaiting_grace && let Some(generation) = generation {
                self.schedule_force_stop(pane, generation);
            }
        } else {
            // No child will exit here; cancel the pending respawn and stop now.
            self.bump_generation(pane);
            self.deactivate(pane);
            self.workspace.set_state(pane, ProcessState::Exited);
        }
    }

    /// The pane id of the currently selected process.
    fn selected_pane(&self) -> Option<PaneId> {
        self.workspace
            .selected_process()
            .map(|process| *process.id())
    }

    /// Resizes every live pane's PTY and parser to match `area`.
    fn resize(&mut self, area: Rect) {
        self.pane_size = pane_size_of(area);
        let rows = self.pane_size.rows().into_inner();
        let cols = self.pane_size.cols().into_inner();
        let size = self.pane_size;
        for pane in self.panes.values_mut() {
            pane.parser.screen_mut().set_size(rows, cols);
            if let Some(handle) = pane.handle.as_mut() {
                let _ = handle.resize(size);
            }
        }
    }
}

/// Serves directory-completion requests off the event loop: coalesces to the
/// newest pending request, reads the filesystem, and returns candidates as a
/// generation-tagged event. Exits when the request or event channel closes.
fn completion_worker(
    completer: Box<dyn PathCompleter + Send>,
    requests: Receiver<CompletionRequest>,
    events: Sender<RuntimeEvent>,
) {
    while let Ok(mut request) = requests.recv() {
        while let Ok(newer) = requests.try_recv() {
            request = newer;
        }
        let candidates = completer.complete_dir(&request.partial);
        if events
            .send(RuntimeEvent::Completions {
                generation: request.generation,
                candidates,
            })
            .is_err()
        {
            break;
        }
    }
}

/// Identifies a process's config spec by its full resolved identity: the same
/// tuple reconciliation matches on, including the effective autostart. Matching
/// on the resolved autostart is what distinguishes a spec from an otherwise
/// identical sibling once one of them is edited, so the right spec is found
/// regardless of the process's row position after a reconcile.
struct SpecMatch {
    kind: ProcessKind,
    name: ProcessName,
    command: Option<CommandLine>,
    working_dir: Option<PathBuf>,
    description: Option<Description>,
    restart: RestartPolicy,
    autostart: bool,
}

impl SpecMatch {
    /// The match key for `process`'s config spec, taken from its current state.
    fn of(process: &Process) -> Self {
        Self {
            kind: *process.kind(),
            name: process.name().clone(),
            command: process.command().clone(),
            working_dir: process.working_dir().clone(),
            description: process.description().clone(),
            restart: *process.restart(),
            autostart: *process.autostart(),
        }
    }

    /// Whether `spec` resolves to the same identity as the identified process.
    fn matches(&self, spec: &ProcessSpec) -> bool {
        spec.name() == &self.name
            && spec.command() == &self.command
            && spec.working_dir() == &self.working_dir
            && spec.description() == &self.description
            && spec.restart_policy() == self.restart
            && spec.should_autostart(self.kind) == self.autostart
    }

    /// Whether `process` shares this full identity. Used to count which of any
    /// identical rows the selected one is, so the same-numbered identical spec is
    /// edited (identical specs are the only case where more than one can match).
    fn matches_process(&self, process: &Process) -> bool {
        *process.kind() == self.kind
            && process.name() == &self.name
            && process.command() == &self.command
            && process.working_dir() == &self.working_dir
            && process.description() == &self.description
            && *process.restart() == self.restart
            && *process.autostart() == self.autostart
    }

    /// Returns `config` with the autostart of the `occurrence`-th matching spec
    /// set to `autostart`, plus whether that spec existed. Identical specs are
    /// numbered so the selected live row maps back to the same config row.
    fn with_autostart(
        &self,
        config: WorkspaceConfig,
        occurrence: usize,
        autostart: Option<bool>,
    ) -> (WorkspaceConfig, bool) {
        let mut seen = 0;
        let mut edited = false;
        let mut apply = |specs: &[ProcessSpec]| -> Vec<ProcessSpec> {
            specs
                .iter()
                .map(|spec| {
                    if self.matches(spec) {
                        let hit = seen == occurrence;
                        seen += 1;
                        if hit {
                            edited = true;
                            return spec.clone().with_autostart(autostart);
                        }
                    }
                    spec.clone()
                })
                .collect()
        };
        let config = match self.kind {
            ProcessKind::Agent => {
                let specs = apply(config.agents());
                config.with_agents(specs)
            },
            ProcessKind::Terminal => {
                let specs = apply(config.terminals());
                config.with_terminals(specs)
            },
            ProcessKind::Command => {
                let specs = apply(config.commands());
                config.with_commands(specs)
            },
        };
        (config, edited)
    }
}

/// A display name for a project taken from its config path: the parent
/// directory's name, else the app name. The path is normalized first so a
/// relative default like `muster.yml` resolves to the current directory's name
/// rather than losing its (empty) parent.
fn label_from_config(config: &Path) -> String {
    path::normalize(config)
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| APP_NAME.to_string())
}

/// Whether `key` is the leader chord (Control + the leader key).
fn is_leader(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(LEADER_KEY)
}

/// Splits a frame into (sidebar, main, status) rectangles.
fn areas(area: Rect) -> (Rect, Rect, Rect) {
    let body =
        Layout::vertical([Constraint::Min(1), Constraint::Length(STATUS_BAR_HEIGHT)]).split(area);
    let columns =
        Layout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(1)]).split(body[0]);
    (columns[0], columns[1], body[1])
}

/// The PTY size of the main terminal pane for a given frame `area`.
fn pane_size_of(area: Rect) -> PtySize {
    let (_, main, _) = areas(area);
    let inner = BORDER_THICKNESS * 2;
    let rows = main.height.saturating_sub(inner).max(MIN_PANE_DIMENSION);
    let cols = main.width.saturating_sub(inner).max(MIN_PANE_DIMENSION);
    PtySize::builder()
        .rows(Rows::new(rows))
        .cols(Cols::new(cols))
        .build()
}

/// Maps an exit outcome to the resting lifecycle state.
fn exit_state(outcome: ExitOutcome) -> ProcessState {
    match outcome {
        ExitOutcome::Succeeded => ProcessState::Exited,
        ExitOutcome::Failed => ProcessState::Crashed,
    }
}

/// A starter workspace for a new project: a single terminal running the login
/// shell, so the project is immediately usable.
fn starter_config() -> WorkspaceConfig {
    let terminals = ProcessName::try_new(STARTER_TERMINAL)
        .map(|name| vec![ProcessSpec::builder().name(name).build()])
        .unwrap_or_default();
    WorkspaceConfig::builder()
        .agents(vec![])
        .terminals(terminals)
        .commands(vec![])
        .build()
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        path::Path,
        rc::Rc,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use crossbeam_channel::bounded;

    use super::*;
    use crate::{
        adapter::tui::activity::OUTPUT_IDLE_TIMEOUT,
        domain::{
            config::{ConfigError, ProcessSpec},
            port::OutputSink,
            process::{Process, ProcessKind, RestartPolicy},
            pty::PtyError,
            value::{ProcessName, ProjectName},
        },
    };

    /// A completer that suggests nothing.
    struct FakeCompleter;

    impl PathCompleter for FakeCompleter {
        fn complete_dir(&self, _partial: &str) -> Vec<String> {
            Vec::new()
        }
    }

    /// A completer that always returns the same candidates.
    struct CannedCompleter(Vec<String>);

    impl PathCompleter for CannedCompleter {
        fn complete_dir(&self, _partial: &str) -> Vec<String> {
            self.0.clone()
        }
    }

    /// A shared record of what a `FakeRegistry` was asked to persist.
    #[derive(Clone, Default)]
    struct Recorder {
        projects: Rc<RefCell<Option<Vec<Project>>>>,
        workspaces: Rc<RefCell<Vec<(PathBuf, WorkspaceConfig)>>>,
    }

    /// A registry returning a fixed project list and one workspace config, and
    /// recording everything it is asked to save.
    struct FakeRegistry {
        projects: Vec<Project>,
        workspace: WorkspaceConfig,
        recorder: Recorder,
    }

    impl ProjectRegistry for FakeRegistry {
        fn projects(&self) -> Result<Vec<Project>, ConfigError> {
            Ok(self
                .recorder
                .projects
                .borrow()
                .clone()
                .unwrap_or_else(|| self.projects.clone()))
        }

        fn workspace(&self, _config_path: &Path) -> Result<WorkspaceConfig, ConfigError> {
            Ok(self.workspace.clone())
        }

        fn workspace_exists(&self, config_path: &Path) -> bool {
            self.recorder
                .workspaces
                .borrow()
                .iter()
                .any(|(path, _)| path == config_path)
        }

        fn save(&self, projects: &[Project]) -> Result<(), ConfigError> {
            *self.recorder.projects.borrow_mut() = Some(projects.to_vec());
            Ok(())
        }

        fn save_workspace(
            &self,
            config_path: &Path,
            config: &WorkspaceConfig,
        ) -> Result<(), ConfigError> {
            self.recorder
                .workspaces
                .borrow_mut()
                .push((config_path.to_path_buf(), config.clone()));
            Ok(())
        }
    }

    /// A workspace config with no processes in any section.
    fn empty_workspace_config() -> WorkspaceConfig {
        WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![])
            .build()
    }

    /// A registry with no registered projects.
    fn empty_registry() -> Box<dyn ProjectRegistry> {
        Box::new(FakeRegistry {
            projects: Vec::new(),
            workspace: empty_workspace_config(),
            recorder: Recorder::default(),
        })
    }

    /// A runner whose spawned processes never emit output and never exit on
    /// their own; the test drives exits explicitly via `handle_output`.
    struct FakeRunner;

    impl ProcessRunner for FakeRunner {
        fn spawn(
            &self,
            _request: SpawnRequest,
            _sink: Box<dyn OutputSink>,
        ) -> Result<Box<dyn ProcessHandle>, PtyError> {
            Ok(Box::new(FakeHandle))
        }
    }

    struct FakeHandle;

    impl ProcessHandle for FakeHandle {
        fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
            Ok(())
        }

        fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
            Ok(())
        }

        fn pause(&mut self) -> Result<(), PtyError> {
            Ok(())
        }

        fn resume(&mut self) -> Result<(), PtyError> {
            Ok(())
        }

        fn kill(&mut self) -> Result<(), PtyError> {
            Ok(())
        }
    }

    /// A runner that succeeds once, then fails every subsequent spawn.
    struct FlakyRunner {
        spawns: AtomicUsize,
    }

    impl ProcessRunner for FlakyRunner {
        fn spawn(
            &self,
            _request: SpawnRequest,
            _sink: Box<dyn OutputSink>,
        ) -> Result<Box<dyn ProcessHandle>, PtyError> {
            if self.spawns.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(Box::new(FakeHandle))
            } else {
                Err(PtyError::System("spawn failed".to_string()))
            }
        }
    }

    /// A runner whose every spawn fails, modelling a persistent PTY/resource
    /// failure.
    struct FailingRunner;

    impl ProcessRunner for FailingRunner {
        fn spawn(
            &self,
            _request: SpawnRequest,
            _sink: Box<dyn OutputSink>,
        ) -> Result<Box<dyn ProcessHandle>, PtyError> {
            Err(PtyError::System("spawn failed".to_string()))
        }
    }

    /// A handle that records everything written to its PTY.
    struct RecordingHandle {
        written: Arc<Mutex<Vec<u8>>>,
    }

    impl ProcessHandle for RecordingHandle {
        fn write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
            self.written.lock().unwrap().extend_from_slice(bytes);
            Ok(())
        }

        fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
            Ok(())
        }

        fn pause(&mut self) -> Result<(), PtyError> {
            Ok(())
        }

        fn resume(&mut self) -> Result<(), PtyError> {
            Ok(())
        }

        fn kill(&mut self) -> Result<(), PtyError> {
            Ok(())
        }
    }

    /// A runner whose spawned processes record their input.
    struct RecordingRunner {
        written: Arc<Mutex<Vec<u8>>>,
    }

    impl ProcessRunner for RecordingRunner {
        fn spawn(
            &self,
            _request: SpawnRequest,
            _sink: Box<dyn OutputSink>,
        ) -> Result<Box<dyn ProcessHandle>, PtyError> {
            Ok(Box::new(RecordingHandle {
                written: self.written.clone(),
            }))
        }
    }

    /// A runner whose handles count how many times they are killed.
    struct KillCountRunner {
        kills: Arc<AtomicUsize>,
    }

    impl ProcessRunner for KillCountRunner {
        fn spawn(
            &self,
            _request: SpawnRequest,
            _sink: Box<dyn OutputSink>,
        ) -> Result<Box<dyn ProcessHandle>, PtyError> {
            Ok(Box::new(KillCountHandle {
                kills: self.kills.clone(),
            }))
        }
    }

    struct KillCountHandle {
        kills: Arc<AtomicUsize>,
    }

    impl ProcessHandle for KillCountHandle {
        fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
            Ok(())
        }

        fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
            Ok(())
        }

        fn pause(&mut self) -> Result<(), PtyError> {
            Ok(())
        }

        fn resume(&mut self) -> Result<(), PtyError> {
            Ok(())
        }

        fn kill(&mut self) -> Result<(), PtyError> {
            self.kills.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// Counts graceful termination requests separately from hard kills.
    #[derive(Default)]
    struct StopSignals {
        terminates: AtomicUsize,
        kills: AtomicUsize,
    }

    /// A runner whose handles record which stop mechanism the app selected.
    struct StopSignalRunner {
        signals: Arc<StopSignals>,
    }

    impl ProcessRunner for StopSignalRunner {
        fn spawn(
            &self,
            _request: SpawnRequest,
            _sink: Box<dyn OutputSink>,
        ) -> Result<Box<dyn ProcessHandle>, PtyError> {
            Ok(Box::new(StopSignalHandle {
                signals: self.signals.clone(),
            }))
        }
    }

    /// A handle that distinguishes graceful termination from a hard kill.
    struct StopSignalHandle {
        signals: Arc<StopSignals>,
    }

    impl ProcessHandle for StopSignalHandle {
        fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
            Ok(())
        }

        fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
            Ok(())
        }

        fn pause(&mut self) -> Result<(), PtyError> {
            Ok(())
        }

        fn resume(&mut self) -> Result<(), PtyError> {
            Ok(())
        }

        fn terminate(&mut self, _grace: Duration) -> Result<(), PtyError> {
            self.signals.terminates.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn kill(&mut self) -> Result<(), PtyError> {
            self.signals.kills.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// Counts stop attempts for a handle whose first hard kill fails.
    #[derive(Default)]
    struct RetryStopSignals {
        terminates: AtomicUsize,
        kills: AtomicUsize,
    }

    /// Configured result of graceful termination in retry tests.
    #[derive(Clone, Copy)]
    enum TerminateOutcome {
        Succeeds,
        Fails,
    }

    /// A runner used to verify that failed stop delivery remains retryable.
    struct RetryStopRunner {
        signals: Arc<RetryStopSignals>,
        terminate_outcome: TerminateOutcome,
    }

    impl ProcessRunner for RetryStopRunner {
        fn spawn(
            &self,
            _request: SpawnRequest,
            _sink: Box<dyn OutputSink>,
        ) -> Result<Box<dyn ProcessHandle>, PtyError> {
            Ok(Box::new(RetryStopHandle {
                signals: self.signals.clone(),
                terminate_outcome: self.terminate_outcome,
            }))
        }
    }

    /// A handle whose graceful termination always fails and whose hard kill
    /// succeeds on the second attempt.
    struct RetryStopHandle {
        signals: Arc<RetryStopSignals>,
        terminate_outcome: TerminateOutcome,
    }

    impl ProcessHandle for RetryStopHandle {
        fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
            Ok(())
        }

        fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
            Ok(())
        }

        fn pause(&mut self) -> Result<(), PtyError> {
            Ok(())
        }

        fn resume(&mut self) -> Result<(), PtyError> {
            Ok(())
        }

        fn terminate(&mut self, _grace: Duration) -> Result<(), PtyError> {
            self.signals.terminates.fetch_add(1, Ordering::SeqCst);
            match self.terminate_outcome {
                TerminateOutcome::Succeeds => Ok(()),
                TerminateOutcome::Fails => Err(PtyError::Unsupported("signal failed".to_string())),
            }
        }

        fn kill(&mut self) -> Result<(), PtyError> {
            let attempt = self.signals.kills.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                Err(PtyError::Unsupported("signal failed".to_string()))
            } else {
                Ok(())
            }
        }
    }

    /// A runner whose handles fail every pause and resume, modelling a signal
    /// that cannot be delivered.
    struct PauseFailRunner;

    impl ProcessRunner for PauseFailRunner {
        fn spawn(
            &self,
            _request: SpawnRequest,
            _sink: Box<dyn OutputSink>,
        ) -> Result<Box<dyn ProcessHandle>, PtyError> {
            Ok(Box::new(PauseFailHandle))
        }
    }

    struct PauseFailHandle;

    impl ProcessHandle for PauseFailHandle {
        fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
            Ok(())
        }

        fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
            Ok(())
        }

        fn pause(&mut self) -> Result<(), PtyError> {
            Err(PtyError::Unsupported("signal failed".to_string()))
        }

        fn resume(&mut self) -> Result<(), PtyError> {
            Err(PtyError::Unsupported("signal failed".to_string()))
        }

        fn kill(&mut self) -> Result<(), PtyError> {
            Ok(())
        }
    }

    const PANE: u64 = 0;

    fn app_with(restart: RestartPolicy) -> App {
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(PANE))
                    .name(ProcessName::try_new("p").unwrap())
                    .kind(ProcessKind::Terminal)
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .restart(restart)
                    .build(),
            ])
            .build();
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            empty_registry(),
            PathBuf::from("muster.yml"),
        );
        app.start();
        app
    }

    /// Builds a live app of `kind` whose handle records graceful and hard stops.
    fn app_with_stop_signals(kind: ProcessKind) -> (App, Arc<StopSignals>) {
        let signals = Arc::new(StopSignals::default());
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(PANE))
                    .name(ProcessName::try_new("p").unwrap())
                    .kind(kind)
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .restart(RestartPolicy::Never)
                    .build(),
            ])
            .build();
        let mut app = App::new(
            workspace,
            Box::new(StopSignalRunner {
                signals: signals.clone(),
            }),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            empty_registry(),
            PathBuf::from("muster.yml"),
        );
        app.start();
        (app, signals)
    }

    /// Builds a live app whose first hard-stop delivery fails and second succeeds.
    fn app_with_retrying_stop_signals(
        kind: ProcessKind,
        terminate_outcome: TerminateOutcome,
        restart: RestartPolicy,
    ) -> (App, Arc<RetryStopSignals>) {
        let signals = Arc::new(RetryStopSignals::default());
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(PANE))
                    .name(ProcessName::try_new("p").unwrap())
                    .kind(kind)
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .restart(restart)
                    .build(),
            ])
            .build();
        let mut app = App::new(
            workspace,
            Box::new(RetryStopRunner {
                signals: signals.clone(),
                terminate_outcome,
            }),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            empty_registry(),
            PathBuf::from("muster.yml"),
        );
        app.start();
        (app, signals)
    }

    /// Builds and starts an app whose runner always fails to spawn.
    fn failing_app(restart: RestartPolicy) -> App {
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(PANE))
                    .name(ProcessName::try_new("p").unwrap())
                    .kind(ProcessKind::Terminal)
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .restart(restart)
                    .build(),
            ])
            .build();
        let mut app = App::new(
            workspace,
            Box::new(FailingRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            empty_registry(),
            PathBuf::from("muster.yml"),
        );
        app.start();
        app
    }

    fn state(app: &App) -> ProcessState {
        *app.workspace.process(PaneId::new(PANE)).unwrap().state()
    }

    fn current_gen(app: &App) -> SpawnGeneration {
        *app.generations.get(&PaneId::new(PANE)).unwrap()
    }

    #[test]
    fn exit_without_restart_keeps_the_parser_and_marks_crashed() {
        let mut app = app_with(RestartPolicy::Never);
        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Exited(ExitOutcome::Failed),
        );

        let pane = app.panes.get(&PaneId::new(PANE)).expect("parser retained");
        assert!(pane.handle.is_none(), "live handle dropped");
        assert_eq!(state(&app), ProcessState::Crashed);
    }

    /// A notifier that records delivered and closed notification identifiers.
    #[derive(Clone, Default)]
    struct RecordingNotifier {
        count: Rc<RefCell<usize>>,
        identifiers: Rc<RefCell<Vec<Option<NotificationId>>>>,
        scopes: Rc<RefCell<Vec<NotificationScope>>>,
        bodies: Rc<RefCell<Vec<Option<String>>>>,
        closed: Rc<RefCell<Vec<(PaneId, NotificationId)>>>,
    }

    impl RecordingNotifier {
        /// Number of notifications delivered.
        fn count(&self) -> usize {
            *self.count.borrow()
        }

        /// Identifiers carried by delivered notifications.
        fn identifiers(&self) -> Vec<Option<NotificationId>> {
            self.identifiers.borrow().clone()
        }

        /// Terminal-lifetime scopes carried by delivered notifications.
        fn scopes(&self) -> Vec<NotificationScope> {
            self.scopes.borrow().clone()
        }

        /// Bodies carried by delivered notifications.
        fn bodies(&self) -> Vec<Option<String>> {
            self.bodies.borrow().clone()
        }

        /// Identifiers passed to close requests.
        fn closed(&self) -> Vec<(PaneId, NotificationId)> {
            self.closed.borrow().clone()
        }
    }

    impl Notifier for RecordingNotifier {
        fn notify(&self, notification: &Notification) {
            *self.count.borrow_mut() += 1;
            self.identifiers
                .borrow_mut()
                .push(notification.identifier().clone());
            self.scopes.borrow_mut().push(*notification.scope());
            self.bodies.borrow_mut().push(notification.body().clone());
        }

        fn close(&self, pane: PaneId, _scope: NotificationScope, identifier: &NotificationId) {
            self.closed.borrow_mut().push((pane, identifier.clone()));
        }
    }

    fn activity(app: &App) -> ActivityState {
        *app.workspace.process(PaneId::new(PANE)).unwrap().activity()
    }

    /// Whether a confirmation dialog is the active overlay.
    fn confirmation_open(app: &App) -> bool {
        matches!(
            &app.overlay,
            Some(Overlay::ConfirmOverwrite { .. } | Overlay::ConfirmRemoval { .. })
        )
    }

    /// Whether the full-keymap help is the active overlay.
    fn help_open(app: &App) -> bool {
        matches!(&app.overlay, Some(Overlay::Help))
    }

    /// Turns desktop notifications on in-memory, as a loaded settings file would.
    fn enable_desktop(app: &mut App) {
        app.set_settings_store(Box::new(FakeSettingsStore::default()));
    }

    /// A settings store that loads desktop-on and records what it was asked to save.
    #[derive(Clone, Default)]
    struct FakeSettingsStore {
        saved: Rc<RefCell<Option<bool>>>,
    }

    impl SettingsStore for FakeSettingsStore {
        fn load(&self) -> Result<Settings, ConfigError> {
            Ok(Settings::builder().desktop_notifications(true).build())
        }

        fn save(&self, settings: &Settings) -> Result<(), ConfigError> {
            *self.saved.borrow_mut() = Some(*settings.desktop_notifications());
            Ok(())
        }
    }

    /// A store that simulates an existing malformed settings file and records
    /// whether the app tried to replace it.
    #[derive(Clone, Default)]
    struct MalformedSettingsStore {
        save_attempted: Rc<RefCell<bool>>,
    }

    impl SettingsStore for MalformedSettingsStore {
        fn load(&self) -> Result<Settings, ConfigError> {
            let error = serde_yaml_ng::from_str::<Settings>("desktop_notifications: [")
                .expect_err("fixture must remain malformed");
            Err(ConfigError::Parse(error))
        }

        fn save(&self, _settings: &Settings) -> Result<(), ConfigError> {
            *self.save_attempted.borrow_mut() = true;
            Ok(())
        }
    }

    #[test]
    fn output_marks_the_process_working() {
        let mut app = app_with(RestartPolicy::Never);
        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Chunk(b"compiling...".to_vec()),
        );
        assert_eq!(activity(&app), ActivityState::Working);
    }

    #[test]
    fn ordinary_output_returns_to_idle_after_becoming_quiet() {
        let mut app = app_with(RestartPolicy::Never);
        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Chunk(b"prompt> ".to_vec()),
        );
        let deadline = app
            .next_activity_deadline()
            .expect("ordinary output schedules an idle transition");

        assert!(app.expire_quiet_activity(deadline));
        assert_eq!(activity(&app), ActivityState::Idle);
        assert!(app.next_activity_deadline().is_none());
    }

    #[test]
    fn explicit_progress_stays_working_until_a_protocol_update() {
        let mut app = app_with(RestartPolicy::Never);
        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Chunk(b"\x1b]9;4;1;40\x07".to_vec()),
        );
        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Chunk(b"still working".to_vec()),
        );

        assert!(app.next_activity_deadline().is_none());
        assert!(!app.expire_quiet_activity(Instant::now() + OUTPUT_IDLE_TIMEOUT));
        assert_eq!(activity(&app), ActivityState::Working);
    }

    #[test]
    fn a_bell_marks_the_process_awaiting_input_and_notifies() {
        let mut app = app_with(RestartPolicy::Never);
        let notifier = RecordingNotifier::default();
        app.set_notifier(Box::new(notifier.clone()));
        enable_desktop(&mut app);

        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Chunk(b"\x07".to_vec()),
        );

        assert_eq!(activity(&app), ActivityState::AwaitingInput);
        assert!(app.notice.is_some(), "an in-app notice is raised");
        assert_eq!(notifier.count(), 1, "the desktop notifier is called");
        assert_eq!(notifier.bodies(), [Some(AWAITING_INPUT_NOTICE.to_string())]);
    }

    #[test]
    fn a_title_only_notification_has_no_duplicate_desktop_body() {
        let mut app = app_with(RestartPolicy::Never);
        let notifier = RecordingNotifier::default();
        app.set_notifier(Box::new(notifier.clone()));
        enable_desktop(&mut app);

        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Chunk(b"\x1b]99;;Hello world\x1b\\".to_vec()),
        );

        assert!(
            app.notice
                .as_deref()
                .is_some_and(|notice| notice.contains("Hello world"))
        );
        assert_eq!(notifier.bodies(), [None]);
    }

    #[test]
    fn an_st_terminated_notification_keeps_the_awaiting_marker() {
        // kitty/other OSC notifications close with ST (`ESC \`); the terminator
        // must not flip the pane back to working and lose its attention marker.
        let mut app = app_with(RestartPolicy::Never);
        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Chunk(b"\x1b]777;notify;agent;done\x1b\\".to_vec()),
        );
        assert_eq!(activity(&app), ActivityState::AwaitingInput);
    }

    #[test]
    fn a_burst_of_bells_is_throttled_to_one_notification() {
        let mut app = app_with(RestartPolicy::Never);
        let notifier = RecordingNotifier::default();
        app.set_notifier(Box::new(notifier.clone()));
        enable_desktop(&mut app);

        for _ in 0..5 {
            app.handle_output(
                PaneId::new(PANE),
                current_gen(&app),
                ProcessOutput::Chunk(b"\x07".to_vec()),
            );
        }
        assert_eq!(notifier.count(), 1, "a bell burst collapses to one alert");
    }

    #[test]
    fn kitty_update_and_close_identifiers_reach_the_notifier() {
        let mut app = app_with(RestartPolicy::Never);
        let notifier = RecordingNotifier::default();
        app.set_notifier(Box::new(notifier.clone()));
        enable_desktop(&mut app);

        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Chunk(
                b"\x1b]99;i=build;first\x1b\\\x1b]99;i=build;second\x1b\\\x1b]99;i=build:p=close;\x1b\\"
                    .to_vec(),
            ),
        );

        assert_eq!(
            notifier
                .identifiers()
                .into_iter()
                .flatten()
                .map(|identifier| identifier.to_string())
                .collect::<Vec<_>>(),
            ["build", "build"]
        );
        assert_eq!(
            notifier
                .closed()
                .into_iter()
                .map(|(pane, identifier)| (pane, identifier.to_string()))
                .collect::<Vec<_>>(),
            [(PaneId::new(PANE), "build".to_string())]
        );
    }

    #[test]
    fn a_reused_pane_gets_a_new_notification_scope_after_project_switch() {
        let mut app = app_with(RestartPolicy::Never);
        let notifier = RecordingNotifier::default();
        app.set_notifier(Box::new(notifier.clone()));
        enable_desktop(&mut app);

        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Chunk(b"\x1b]99;i=build;old project\x1b\\".to_vec()),
        );
        app.load_project(
            one_agent_config("new project"),
            PathBuf::from("new-project.yml"),
        );
        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Chunk(b"\x1b]99;i=build;new project\x1b\\".to_vec()),
        );

        let scopes = notifier.scopes();
        assert_eq!(scopes.len(), 2);
        assert_ne!(scopes[0], scopes[1]);
    }

    #[test]
    fn desktop_off_suppresses_the_notification_but_not_the_notice() {
        let mut app = app_with(RestartPolicy::Never);
        let notifier = RecordingNotifier::default();
        app.set_notifier(Box::new(notifier.clone()));
        app.settings = SettingsState::Loaded {
            settings: Settings::builder().desktop_notifications(false).build(),
            store: Box::new(FakeSettingsStore::default()),
        };

        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Chunk(b"\x07".to_vec()),
        );

        assert!(app.notice.is_some(), "the in-app notice still shows");
        assert_eq!(
            notifier.count(),
            0,
            "no desktop notification while the setting is off"
        );
    }

    #[test]
    fn toggling_notifications_flips_and_persists_the_setting() {
        let mut app = app_with(RestartPolicy::Never);
        let store = FakeSettingsStore::default();
        app.set_settings_store(Box::new(store.clone()));
        assert!(app.desktop_notifications_enabled(), "loads desktop-on");

        press(&mut app, KeyCode::Char('N'));

        assert!(
            !app.desktop_notifications_enabled(),
            "the toggle flips the live setting"
        );
        assert_eq!(
            *store.saved.borrow(),
            Some(false),
            "the new value is persisted"
        );
    }

    #[test]
    fn a_settings_load_error_blocks_the_toggle_without_saving() {
        let mut app = app_with(RestartPolicy::Never);
        let store = MalformedSettingsStore::default();
        app.set_settings_store(Box::new(store.clone()));
        assert!(matches!(app.settings, SettingsState::LoadFailed(_)));

        press(&mut app, KeyCode::Char('N'));

        assert!(
            !*store.save_attempted.borrow(),
            "the malformed file must not be replaced"
        );
        assert!(
            app.notice
                .as_deref()
                .is_some_and(|notice| notice.contains(SETTINGS_LOAD_ERROR)),
            "the load failure is reported when the toggle is refused"
        );
    }

    #[test]
    fn a_restart_clears_the_awaiting_input_marker() {
        let mut app = app_with(RestartPolicy::Always);
        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Chunk(b"\x07".to_vec()),
        );
        assert_eq!(activity(&app), ActivityState::AwaitingInput);

        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Exited(ExitOutcome::Failed),
        );
        assert_eq!(state(&app), ProcessState::Restarting);
        assert_eq!(
            activity(&app),
            ActivityState::Idle,
            "a process in restart backoff shows no attention marker"
        );
    }

    #[test]
    fn spawn_failure_under_auto_restart_stays_in_backoff() {
        let app = failing_app(RestartPolicy::Always);

        // A transient spawn failure under an auto-restart policy must schedule a
        // backoff retry, not permanently abandon the process as crashed.
        assert_eq!(state(&app), ProcessState::Restarting);
        assert!(
            app.restart_attempts.contains_key(&PaneId::new(PANE)),
            "a backoff retry must be scheduled after a failed spawn"
        );
    }

    #[test]
    fn spawn_failure_without_restart_is_crashed() {
        let app = failing_app(RestartPolicy::Never);
        assert_eq!(state(&app), ProcessState::Crashed);
    }

    #[test]
    fn pause_then_resume_toggles_state_while_the_process_stays_alive() {
        let mut app = app_with(RestartPolicy::Never);
        assert_eq!(state(&app), ProcessState::Running);

        app.toggle_pause_selected();
        assert_eq!(state(&app), ProcessState::Paused);
        assert!(
            app.panes.get(&PaneId::new(PANE)).unwrap().handle.is_some(),
            "a paused process keeps its live handle"
        );

        app.toggle_pause_selected();
        assert_eq!(state(&app), ProcessState::Running);
    }

    #[test]
    fn pausing_a_finished_process_is_a_noop() {
        let mut app = app_with(RestartPolicy::Never);
        app.stop_selected();
        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Exited(ExitOutcome::Succeeded),
        );
        assert_eq!(state(&app), ProcessState::Exited);

        app.toggle_pause_selected();
        assert_eq!(
            state(&app),
            ProcessState::Exited,
            "there is no live process to pause"
        );
    }

    #[test]
    fn a_failed_pause_signal_leaves_the_state_unchanged() {
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(PANE))
                    .name(ProcessName::try_new("p").unwrap())
                    .kind(ProcessKind::Terminal)
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .restart(RestartPolicy::Never)
                    .build(),
            ])
            .build();
        let mut app = App::new(
            workspace,
            Box::new(PauseFailRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            empty_registry(),
            PathBuf::from("muster.yml"),
        );
        app.start();
        assert_eq!(state(&app), ProcessState::Running);

        app.toggle_pause_selected();

        // The signal failed, so the UI must not claim the process is paused.
        assert_eq!(
            state(&app),
            ProcessState::Running,
            "a failed pause signal must not flip the state"
        );
    }

    #[test]
    fn user_stop_is_exited_not_crashed_even_on_nonzero_exit() {
        let mut app = app_with(RestartPolicy::Always);
        app.stop_selected();
        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Exited(ExitOutcome::Failed),
        );

        assert_eq!(state(&app), ProcessState::Exited);
        assert!(app.panes.get(&PaneId::new(PANE)).unwrap().handle.is_none());
    }

    #[test]
    fn repeated_manual_command_stop_reuses_one_graceful_escalation() {
        let (mut app, signals) = app_with_stop_signals(ProcessKind::Command);
        app.stop_selected();
        app.stop_selected();

        assert_eq!(signals.terminates.load(Ordering::SeqCst), 1);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 0);

        app.handle_force_stop(PaneId::new(PANE), current_gen(&app));
        assert_eq!(signals.kills.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_command_stop_can_be_retried_after_both_signals_fail() {
        let (mut app, signals) = app_with_retrying_stop_signals(
            ProcessKind::Command,
            TerminateOutcome::Fails,
            RestartPolicy::Never,
        );

        app.stop_selected();
        assert_eq!(
            app.panes.get(&PaneId::new(PANE)).unwrap().exit_intent,
            ExitIntent::StopRetryable
        );

        app.stop_selected();
        app.stop_selected();

        assert_eq!(
            app.panes.get(&PaneId::new(PANE)).unwrap().exit_intent,
            ExitIntent::StopInFlight
        );
        assert_eq!(signals.terminates.load(Ordering::SeqCst), 2);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_terminal_stop_can_be_retried_after_kill_fails() {
        let (mut app, signals) = app_with_retrying_stop_signals(
            ProcessKind::Terminal,
            TerminateOutcome::Fails,
            RestartPolicy::Never,
        );

        app.stop_selected();
        assert_eq!(
            app.panes.get(&PaneId::new(PANE)).unwrap().exit_intent,
            ExitIntent::StopRetryable
        );

        app.stop_selected();
        app.stop_selected();

        assert_eq!(
            app.panes.get(&PaneId::new(PANE)).unwrap().exit_intent,
            ExitIntent::StopInFlight
        );
        assert_eq!(signals.terminates.load(Ordering::SeqCst), 0);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_command_stop_can_be_retried_after_escalation_fails() {
        let (mut app, signals) = app_with_retrying_stop_signals(
            ProcessKind::Command,
            TerminateOutcome::Succeeds,
            RestartPolicy::Never,
        );
        let pane = PaneId::new(PANE);

        app.stop_selected();
        app.handle_force_stop(pane, current_gen(&app));
        assert_eq!(
            app.panes.get(&pane).unwrap().exit_intent,
            ExitIntent::StopRetryable
        );

        app.stop_selected();
        app.handle_force_stop(pane, current_gen(&app));

        assert_eq!(
            app.panes.get(&pane).unwrap().exit_intent,
            ExitIntent::StopInFlight
        );
        assert_eq!(signals.terminates.load(Ordering::SeqCst), 2);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn failed_stop_delivery_still_suppresses_restart_when_the_child_exits() {
        let (mut app, _signals) = app_with_retrying_stop_signals(
            ProcessKind::Command,
            TerminateOutcome::Fails,
            RestartPolicy::Always,
        );
        let pane = PaneId::new(PANE);

        app.stop_selected();
        app.handle_output(
            pane,
            current_gen(&app),
            ProcessOutput::Exited(ExitOutcome::Failed),
        );

        assert_eq!(state(&app), ProcessState::Exited);
        assert!(app.panes.get(&pane).unwrap().handle.is_none());
    }

    #[test]
    fn a_manual_terminal_stop_remains_a_hard_kill() {
        let (mut app, signals) = app_with_stop_signals(ProcessKind::Terminal);
        app.stop_selected();

        assert_eq!(signals.terminates.load(Ordering::SeqCst), 0);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_project_switch_hard_kills_commands() {
        let (mut app, signals) = app_with_stop_signals(ProcessKind::Command);
        app.begin_switch(empty_workspace_config(), PathBuf::from("next.yml"));

        assert_eq!(signals.terminates.load(Ordering::SeqCst), 0);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn restart_after_stop_wins_before_the_exit_arrives() {
        // Stop sends an async kill; the user hits restart before the exit lands.
        // The restart is the newer action and must win, not be swallowed by the
        // still-pending stop. Never policy makes the outcome unambiguous: only a
        // force-restart, not the policy, can respawn here.
        let mut app = app_with(RestartPolicy::Never);
        app.stop_selected();
        app.restart_selected();
        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Exited(ExitOutcome::Failed),
        );

        assert_eq!(
            state(&app),
            ProcessState::Running,
            "the later restart must win over the pending stop"
        );
        assert!(
            app.panes.get(&PaneId::new(PANE)).unwrap().handle.is_some(),
            "the process was respawned"
        );
    }

    #[test]
    fn stale_respawn_after_manual_restart_is_ignored() {
        let mut app = app_with(RestartPolicy::Always);
        // Exit schedules a backoff restart: the pane deactivates and its
        // generation is bumped to the value the pending respawn carries.
        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Exited(ExitOutcome::Failed),
        );
        let stale = *app.generations.get(&PaneId::new(PANE)).unwrap();
        assert!(app.panes.get(&PaneId::new(PANE)).unwrap().handle.is_none());

        // A manual restart during the backoff respawns now and advances the gen.
        app.restart_selected();
        let current = *app.generations.get(&PaneId::new(PANE)).unwrap();
        assert_ne!(current, stale);
        assert!(app.panes.get(&PaneId::new(PANE)).unwrap().handle.is_some());

        // The late scheduled respawn (stale generation) must be a no-op: no new
        // process, no generation change.
        app.handle_respawn(PaneId::new(PANE), stale);
        assert_eq!(*app.generations.get(&PaneId::new(PANE)).unwrap(), current);
        assert!(app.panes.get(&PaneId::new(PANE)).unwrap().handle.is_some());
    }

    #[test]
    fn failed_forced_respawn_deactivates_the_pane() {
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(PANE))
                    .name(ProcessName::try_new("p").unwrap())
                    .kind(ProcessKind::Terminal)
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .restart(RestartPolicy::Never)
                    .build(),
            ])
            .build();
        let mut app = App::new(
            workspace,
            Box::new(FlakyRunner {
                spawns: AtomicUsize::new(0),
            }),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            empty_registry(),
            PathBuf::from("muster.yml"),
        );
        app.start();
        assert!(app.panes.get(&PaneId::new(PANE)).unwrap().handle.is_some());

        // Force-restart: kill the live process, then the forced respawn fails.
        app.restart_selected();
        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Exited(ExitOutcome::Failed),
        );

        // A failed respawn must not leave the dead handle behind, or the UI would
        // treat the pane as alive and never be able to start it again.
        assert!(
            app.panes
                .get(&PaneId::new(PANE))
                .is_none_or(|p| p.handle.is_none())
        );
        assert_eq!(state(&app), ProcessState::Crashed);
    }

    #[test]
    fn double_leader_forwards_ctrl_a_to_the_terminal() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(PANE))
                    .name(ProcessName::try_new("p").unwrap())
                    .kind(ProcessKind::Terminal)
                    .command(Some(CommandLine::try_new("sh").unwrap()))
                    .build(),
            ])
            .build();
        let mut app = App::new(
            workspace,
            Box::new(RecordingRunner {
                written: written.clone(),
            }),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            empty_registry(),
            PathBuf::from("muster.yml"),
        );
        app.start();
        app.focus = Focus::Terminal;

        let ctrl_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        app.handle_key(ctrl_a); // begins the leader chord
        app.handle_key(ctrl_a); // second press forwards a literal Ctrl-A

        assert_eq!(*written.lock().unwrap(), vec![0x01]);
    }

    #[test]
    fn repeated_key_is_forwarded_to_the_terminal() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(PANE))
                    .name(ProcessName::try_new("p").unwrap())
                    .kind(ProcessKind::Terminal)
                    .command(Some(CommandLine::try_new("sh").unwrap()))
                    .build(),
            ])
            .build();
        let mut app = App::new(
            workspace,
            Box::new(RecordingRunner {
                written: written.clone(),
            }),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            empty_registry(),
            PathBuf::from("muster.yml"),
        );
        app.start();
        app.focus = Focus::Terminal;

        let repeat =
            KeyEvent::new_with_kind(KeyCode::Char('a'), KeyModifiers::NONE, KeyEventKind::Repeat);
        app.handle_key(repeat);

        assert_eq!(*written.lock().unwrap(), b"a".to_vec());
    }

    #[test]
    fn pane_size_never_below_the_vt100_safe_minimum() {
        let size = pane_size_of(Rect::new(0, 0, 0, 0));
        assert!(size.rows().into_inner() >= MIN_PANE_DIMENSION);
        assert!(size.cols().into_inner() >= MIN_PANE_DIMENSION);
    }

    #[test]
    fn shutdown_kills_every_live_process() {
        let kills = Arc::new(AtomicUsize::new(0));
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(0))
                    .name(ProcessName::try_new("a").unwrap())
                    .kind(ProcessKind::Terminal)
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .build(),
                Process::builder()
                    .id(PaneId::new(1))
                    .name(ProcessName::try_new("b").unwrap())
                    .kind(ProcessKind::Terminal)
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .build(),
            ])
            .build();
        let mut app = App::new(
            workspace,
            Box::new(KillCountRunner {
                kills: kills.clone(),
            }),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            empty_registry(),
            PathBuf::from("muster.yml"),
        );
        app.start();
        app.shutdown();
        assert_eq!(kills.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn output_from_a_superseded_generation_is_discarded() {
        let mut app = app_with(RestartPolicy::Never);
        let old_gen = current_gen(&app);

        // Restart the live process: its exit drives a fresh spawn (new generation).
        app.restart_selected();
        app.handle_output(
            PaneId::new(PANE),
            old_gen,
            ProcessOutput::Exited(ExitOutcome::Failed),
        );
        let new_gen = current_gen(&app);
        assert_ne!(new_gen, old_gen);

        // A late chunk from the old child is dropped; the current child's applies.
        app.handle_output(
            PaneId::new(PANE),
            old_gen,
            ProcessOutput::Chunk(b"stale".to_vec()),
        );
        app.handle_output(
            PaneId::new(PANE),
            new_gen,
            ProcessOutput::Chunk(b"fresh".to_vec()),
        );

        let screen = app
            .panes
            .get(&PaneId::new(PANE))
            .unwrap()
            .parser
            .screen()
            .contents();
        assert!(screen.contains("fresh"));
        assert!(!screen.contains("stale"));
    }

    /// A registry whose every workspace load fails.
    struct FailingRegistry {
        projects: Vec<Project>,
    }

    impl ProjectRegistry for FailingRegistry {
        fn projects(&self) -> Result<Vec<Project>, ConfigError> {
            Ok(self.projects.clone())
        }

        fn workspace(&self, _config_path: &Path) -> Result<WorkspaceConfig, ConfigError> {
            // The file exists (see workspace_exists) but cannot be parsed - a
            // present-but-broken config, which keeps the switcher open with an
            // error rather than offering to remove the entry.
            Err(ConfigError::Read {
                path: PathBuf::from("/broken/muster.yml"),
                source: std::io::Error::from(std::io::ErrorKind::InvalidData),
            })
        }

        fn workspace_exists(&self, _config_path: &Path) -> bool {
            true
        }

        fn save(&self, _projects: &[Project]) -> Result<(), ConfigError> {
            Ok(())
        }

        fn save_workspace(
            &self,
            _config_path: &Path,
            _config: &WorkspaceConfig,
        ) -> Result<(), ConfigError> {
            Ok(())
        }
    }

    fn project(name: &str, config: &str) -> Project {
        Project::builder()
            .name(ProjectName::try_new(name).unwrap())
            .config(PathBuf::from(config))
            .build()
    }

    /// An app started on a single "p" process, with `projects` registered and
    /// `target` as the config every switch loads. Returns the registry's save
    /// recorder for asserting on writes.
    fn flow_app(projects: Vec<Project>, target: WorkspaceConfig, current: &str) -> (App, Recorder) {
        let recorder = Recorder::default();
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(PANE))
                    .name(ProcessName::try_new("p").unwrap())
                    .kind(ProcessKind::Terminal)
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .restart(RestartPolicy::Never)
                    .build(),
            ])
            .build();
        let registry = Box::new(FakeRegistry {
            projects,
            workspace: target,
            recorder: recorder.clone(),
        });
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            registry,
            PathBuf::from(current),
        );
        app.start();
        (app, recorder)
    }

    #[test]
    fn a_config_change_adds_new_processes_without_touching_running_ones() {
        let config = WorkspaceConfig::builder()
            .agents(vec![])
            // "p" matches the running terminal `flow_app` starts; "q" is new.
            .terminals(vec![
                ProcessSpec::builder()
                    .name(ProcessName::try_new("p").unwrap())
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .build(),
            ])
            .commands(vec![
                ProcessSpec::builder()
                    .name(ProcessName::try_new("q").unwrap())
                    .command(Some(CommandLine::try_new("echo q").unwrap()))
                    .build(),
            ])
            .build();
        let (mut app, _recorder) = flow_app(vec![], config, "/here/muster.yml");
        assert_eq!(
            app.workspace.processes().len(),
            1,
            "starts with the one process"
        );

        app.handle_config_changed(PathBuf::from("/here/muster.yml"));

        let names: Vec<String> = app
            .workspace
            .processes()
            .iter()
            .map(|process| process.name().as_ref().to_string())
            .collect();
        assert_eq!(
            names,
            vec!["p".to_string(), "q".to_string()],
            "the new process is appended"
        );

        let state_of = |name: &str| {
            *app.workspace
                .processes()
                .iter()
                .find(|process| process.name().as_ref() == name)
                .unwrap()
                .state()
        };
        assert_eq!(
            state_of("p"),
            ProcessState::Running,
            "the already-running process is left untouched"
        );
        assert_eq!(
            state_of("q"),
            ProcessState::Pending,
            "the new process appears stopped, not auto-started"
        );
    }

    #[test]
    fn reconciliation_matches_by_full_spec_not_just_name() {
        let command = |name: &str, cmd: &str| {
            ProcessSpec::builder()
                .name(ProcessName::try_new(name).unwrap())
                .command(Some(CommandLine::try_new(cmd).unwrap()))
                .build()
        };
        // The workspace was loaded from a config with a single "npm test".
        let loaded = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![command("npm", "npm test")])
            .build();
        // The config now prepends a same-named command with a different line.
        let updated = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![
                command("npm", "npm run dev"),
                command("npm", "npm test"),
            ])
            .build();

        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(loaded.to_processes())
            .build();
        let registry = Box::new(FakeRegistry {
            projects: vec![],
            workspace: updated,
            recorder: Recorder::default(),
        });
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            registry,
            PathBuf::from("/here/muster.yml"),
        );

        app.handle_config_changed(PathBuf::from("/here/muster.yml"));

        let commands: Vec<String> = app
            .workspace
            .processes()
            .iter()
            .filter_map(|process| process.command().as_ref().map(|c| c.as_ref().to_string()))
            .collect();
        assert_eq!(
            commands,
            vec!["npm test".to_string(), "npm run dev".to_string()],
            "the differently-commanded same-named spec is added; the matching one is not duplicated"
        );
    }

    #[test]
    fn reconciliation_removes_a_stopped_process_dropped_from_config() {
        let command = |name: &str, cmd: &str| {
            ProcessSpec::builder()
                .name(ProcessName::try_new(name).unwrap())
                .command(Some(CommandLine::try_new(cmd).unwrap()))
                .build()
        };
        let loaded = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![command("a", "cmd-a"), command("b", "cmd-b")])
            .build();
        let updated = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![command("a", "cmd-a")])
            .build();
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(loaded.to_processes())
            .build();
        let registry = Box::new(FakeRegistry {
            projects: vec![],
            workspace: updated,
            recorder: Recorder::default(),
        });
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            registry,
            PathBuf::from("/here/muster.yml"),
        );
        // Not started, so both processes are stopped.
        app.handle_config_changed(PathBuf::from("/here/muster.yml"));

        let names: Vec<String> = app
            .workspace
            .processes()
            .iter()
            .map(|process| process.name().as_ref().to_string())
            .collect();
        assert_eq!(
            names,
            vec!["a".to_string()],
            "the stopped process the config dropped is removed"
        );
    }

    #[test]
    fn reconciliation_keeps_a_running_process_absent_from_config() {
        // `flow_app` spawns "p"; a config that omits it must not drop the running
        // process out from under the user.
        let empty = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![])
            .build();
        let (mut app, _recorder) = flow_app(vec![], empty, "/here/muster.yml");

        app.handle_config_changed(PathBuf::from("/here/muster.yml"));

        assert!(
            app.workspace
                .processes()
                .iter()
                .any(|process| process.name().as_ref() == "p"),
            "a running process not in the config is preserved"
        );
    }

    #[test]
    fn reconciliation_replaces_a_stopped_process_whose_command_changed() {
        // The reviewer's case: a stopped command edited from `npm test` to `npm
        // run dev` must replace the old entry, not leave both.
        let spec = |cmd: &str| {
            ProcessSpec::builder()
                .name(ProcessName::try_new("test").unwrap())
                .command(Some(CommandLine::try_new(cmd).unwrap()))
                .build()
        };
        let loaded = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![spec("npm test")])
            .build();
        let updated = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![spec("npm run dev")])
            .build();
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(loaded.to_processes())
            .build();
        let registry = Box::new(FakeRegistry {
            projects: vec![],
            workspace: updated,
            recorder: Recorder::default(),
        });
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            registry,
            PathBuf::from("/here/muster.yml"),
        );
        app.handle_config_changed(PathBuf::from("/here/muster.yml"));

        let commands: Vec<String> = app
            .workspace
            .processes()
            .iter()
            .filter_map(|process| process.command().as_ref().map(|c| c.as_ref().to_string()))
            .collect();
        assert_eq!(
            commands,
            vec!["npm run dev".to_string()],
            "the edited command replaces the old one, leaving no duplicate"
        );
    }

    #[test]
    fn removing_a_project_drops_only_the_target_from_the_registry() {
        let projects = vec![
            project("keep", "/a/muster.yml"),
            project("drop", "/b/muster.yml"),
        ];
        let (mut app, recorder) = flow_app(projects, empty_workspace_config(), "/here/muster.yml");

        app.remove_project(Path::new("/b/muster.yml"));

        let saved = recorder
            .projects
            .borrow()
            .clone()
            .expect("the registry was saved");
        let names: Vec<String> = saved
            .iter()
            .map(|project| project.name().as_ref().to_string())
            .collect();
        assert_eq!(
            names,
            vec!["keep".to_string()],
            "only the targeted project is removed"
        );
    }

    #[test]
    fn a_live_process_removed_from_config_is_retired_when_it_exits() {
        let empty = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![])
            .build();
        let (mut app, _recorder) = flow_app(vec![], empty, "/here/muster.yml");
        let pane = PaneId::new(PANE);

        // "p" is running but the config now omits it: kept for now.
        app.handle_config_changed(PathBuf::from("/here/muster.yml"));
        assert!(
            app.workspace.processes().iter().any(|p| *p.id() == pane),
            "a running process is kept while alive"
        );

        // When it exits, it is retired rather than restart-looping.
        app.handle_exit(pane, ExitOutcome::Succeeded);
        assert!(
            !app.workspace.processes().iter().any(|p| *p.id() == pane),
            "the orphaned process is removed once it exits"
        );
    }

    #[test]
    fn reconciliation_reflects_a_restart_policy_change() {
        let spec = |restart: RestartPolicy| {
            ProcessSpec::builder()
                .name(ProcessName::try_new("w").unwrap())
                .command(Some(CommandLine::try_new("run").unwrap()))
                .restart(Some(restart))
                .build()
        };
        let loaded = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![spec(RestartPolicy::Never)])
            .build();
        let updated = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![spec(RestartPolicy::Always)])
            .build();
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(loaded.to_processes())
            .build();
        let registry = Box::new(FakeRegistry {
            projects: vec![],
            workspace: updated,
            recorder: Recorder::default(),
        });
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            registry,
            PathBuf::from("/here/muster.yml"),
        );
        app.handle_config_changed(PathBuf::from("/here/muster.yml"));

        let policies: Vec<RestartPolicy> = app
            .workspace
            .processes()
            .iter()
            .map(|process| *process.restart())
            .collect();
        assert_eq!(
            policies,
            vec![RestartPolicy::Always],
            "a restart-only edit is reflected, not ignored"
        );
    }

    #[test]
    fn autostart_defaults_by_kind_and_can_be_overridden() {
        let config = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![
                ProcessSpec::builder()
                    .name(ProcessName::try_new("term").unwrap())
                    .build(),
            ])
            .commands(vec![
                ProcessSpec::builder()
                    .name(ProcessName::try_new("cmd").unwrap())
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .build(),
                ProcessSpec::builder()
                    .name(ProcessName::try_new("eager").unwrap())
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .autostart(Some(true))
                    .build(),
            ])
            .build();
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(config.to_processes())
            .build();
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            empty_registry(),
            PathBuf::from("/here/muster.yml"),
        );
        app.start();

        let state = |name: &str| {
            *app.workspace
                .processes()
                .iter()
                .find(|process| process.name().as_ref() == name)
                .unwrap()
                .state()
        };
        assert_eq!(
            state("term"),
            ProcessState::Running,
            "a terminal auto-starts by default"
        );
        assert_eq!(
            state("cmd"),
            ProcessState::Pending,
            "a command stays stopped by default"
        );
        assert_eq!(
            state("eager"),
            ProcessState::Running,
            "autostart: true starts a command anyway"
        );
    }

    #[test]
    fn reconciliation_reflects_an_autostart_change() {
        let spec = |autostart: bool| {
            ProcessSpec::builder()
                .name(ProcessName::try_new("w").unwrap())
                .command(Some(CommandLine::try_new("run").unwrap()))
                .autostart(Some(autostart))
                .build()
        };
        let loaded = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![spec(false)])
            .build();
        let updated = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![spec(true)])
            .build();
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(loaded.to_processes())
            .build();
        let registry = Box::new(FakeRegistry {
            projects: vec![],
            workspace: updated,
            recorder: Recorder::default(),
        });
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            registry,
            PathBuf::from("/here/muster.yml"),
        );
        app.handle_config_changed(PathBuf::from("/here/muster.yml"));

        let autostarts: Vec<bool> = app
            .workspace
            .processes()
            .iter()
            .map(|process| *process.autostart())
            .collect();
        assert_eq!(
            autostarts,
            vec![true],
            "an autostart-only edit is reflected, not collapsed as no change"
        );
    }

    #[test]
    fn a_relative_config_label_resolves_to_the_current_directory_name() {
        let expected = std::env::current_dir()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            label_from_config(Path::new("muster.yml")),
            expected,
            "a bare relative config is labeled by the working directory, not the app name"
        );
        assert_eq!(
            label_from_config(Path::new("/x/proj/muster.yml")),
            "proj",
            "an absolute config uses its parent directory name"
        );
    }

    #[test]
    fn a_config_change_for_a_different_path_is_ignored() {
        let (mut app, _recorder) = flow_app(vec![], one_agent_config("a"), "/here/muster.yml");
        app.handle_config_changed(PathBuf::from("/elsewhere/muster.yml"));
        assert_eq!(
            app.workspace.processes().len(),
            1,
            "a change to some other project's config does not reconcile this one"
        );
    }

    #[test]
    fn the_sidebar_navigates_into_project_rows_and_activates_them() {
        let projects = vec![
            project("one", "/a/muster.yml"),
            project("two", "/b/muster.yml"),
        ];
        let (mut app, _recorder) = flow_app(projects, empty_workspace_config(), "/here/muster.yml");

        assert!(app.project_cursor.is_none(), "starts on the process list");
        press(&mut app, KeyCode::Char('j')); // past the single process onto the first project
        assert_eq!(app.project_cursor, Some(0));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.project_cursor, Some(1));
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.project_cursor, Some(0));
        press(&mut app, KeyCode::Char('h')); // back out to the process list
        assert!(app.project_cursor.is_none());

        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter); // activate the selected project
        assert!(app.project_cursor.is_none(), "activating clears the cursor");
        assert!(
            app.pending_switch.is_some(),
            "a switch into the project was initiated"
        );
    }

    #[test]
    fn the_launched_project_stays_reachable_after_switching_away() {
        // "/here" is not a registered project, yet it must appear in the tree so
        // it can be returned to.
        let projects = vec![project("one", "/a/muster.yml")];
        let (app, _recorder) = flow_app(projects, empty_workspace_config(), "/here/muster.yml");
        assert!(
            app.projects
                .iter()
                .any(|project| project.config() == &PathBuf::from("/here/muster.yml")),
            "the launched config is kept in the project list"
        );
    }

    #[test]
    fn process_actions_are_ignored_when_a_project_row_is_selected() {
        let projects = vec![project("one", "/a/muster.yml")];
        let (mut app, _recorder) = flow_app(projects, empty_workspace_config(), "/here/muster.yml");
        let pane = PaneId::new(PANE);

        press(&mut app, KeyCode::Char('j')); // move the selection onto the project row
        assert_eq!(app.project_cursor, Some(0));

        press(&mut app, KeyCode::Char('x')); // must not touch the active process
        assert_eq!(
            app.panes.get(&pane).unwrap().exit_intent,
            ExitIntent::FollowPolicy,
            "x on a project row leaves the active process running"
        );
    }

    #[test]
    fn d_on_a_project_row_removes_it_after_confirmation() {
        let projects = vec![
            project("alpha", "/a/muster.yml"),
            project("beta", "/b/muster.yml"),
        ];
        let (mut app, recorder) = flow_app(projects, empty_workspace_config(), "/a/muster.yml");

        press(&mut app, KeyCode::Char('j')); // move onto the first other-project row (beta)
        assert_eq!(app.project_cursor, Some(0));

        press(&mut app, KeyCode::Char('d'));
        assert!(confirmation_open(&app), "d asks before removing");
        assert!(
            recorder.projects.borrow().is_none(),
            "nothing is persisted until the confirmation is accepted"
        );

        press(&mut app, KeyCode::Char('y'));
        let saved = recorder
            .projects
            .borrow()
            .clone()
            .expect("the registry was saved");
        assert_eq!(saved.len(), 1);
        assert_eq!(
            saved[0].name().as_ref(),
            "alpha",
            "only the row's project goes"
        );
    }

    #[test]
    fn d_on_the_synthetic_launched_row_is_refused() {
        // Launched on an unregistered config, with one saved project alongside.
        let projects = vec![project("saved", "/saved/muster.yml")];
        let (mut app, recorder) = flow_app(projects, empty_workspace_config(), "/here/muster.yml");
        // Model having switched to the saved project: the launched "/here" config
        // is now a collapsed other-row, synthesized because it was never saved.
        app.current_config = Some(PathBuf::from("/saved/muster.yml"));
        app.refresh_projects();
        assert_eq!(
            app.launched_project_membership,
            LaunchedProjectMembership::Synthetic,
            "the launched config is unsaved"
        );

        press(&mut app, KeyCode::Char('j')); // onto the synthetic launched row
        assert_eq!(app.project_cursor, Some(0));

        press(&mut app, KeyCode::Char('d'));
        assert!(
            !confirmation_open(&app),
            "no removal is offered for an unsaved project"
        );
        assert!(
            app.notice.is_some(),
            "the user is told why nothing happened"
        );
        assert!(
            recorder.projects.borrow().is_none(),
            "the registry is not rewritten"
        );
    }

    #[test]
    fn t_on_a_process_row_toggles_autostart_and_persists_it() {
        let config = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![
                ProcessSpec::builder()
                    .name(ProcessName::try_new("cmd").unwrap())
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .build(),
            ])
            .build();
        let recorder = Recorder::default();
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(config.to_processes())
            .build();
        let registry = Box::new(FakeRegistry {
            projects: vec![],
            workspace: config,
            recorder: recorder.clone(),
        });
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            registry,
            PathBuf::from("/here/muster.yml"),
        );
        app.start();

        assert!(
            !*app.workspace.selected_process().unwrap().autostart(),
            "a command defaults to not auto-starting"
        );

        press(&mut app, KeyCode::Char('t'));

        assert!(
            *app.workspace.selected_process().unwrap().autostart(),
            "t flips the live autostart flag"
        );
        let workspaces = recorder.workspaces.borrow();
        assert_eq!(workspaces.len(), 1, "the change is persisted once");
        assert_eq!(
            workspaces[0].1.commands()[0].autostart(),
            &Some(true),
            "the command spec is saved with autostart on"
        );
    }

    #[test]
    fn t_edits_the_spec_matching_the_selected_row_not_its_position() {
        // Two same-content command specs differing only in autostart, and a
        // workspace whose row order is the reverse of the config order, as a
        // reconcile can leave it once it appends replacement specs. Locating the
        // spec by row position would edit the wrong one; identity must win.
        let spec = |autostart: bool| {
            ProcessSpec::builder()
                .name(ProcessName::try_new("dup").unwrap())
                .command(Some(CommandLine::try_new("true").unwrap()))
                .autostart(Some(autostart))
                .build()
        };
        let config = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![spec(false), spec(true)]) // config order: false, then true
            .build();
        let processes = config.to_processes();
        let recorder = Recorder::default();
        let (sender, _receiver) = bounded(16);
        // Reverse the rows: the already-true process first, the false one second.
        let workspace = Workspace::builder()
            .processes(vec![processes[1].clone(), processes[0].clone()])
            .selected_index(1) // select the false row (config index 0)
            .build();
        let registry = Box::new(FakeRegistry {
            projects: vec![],
            workspace: config,
            recorder: recorder.clone(),
        });
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            registry,
            PathBuf::from("/here/muster.yml"),
        );

        press(&mut app, KeyCode::Char('t'));

        let workspaces = recorder.workspaces.borrow();
        let commands = workspaces[0].1.commands();
        assert_eq!(
            commands[0].autostart(),
            &Some(true),
            "the spec the selected row resolves to (the false one) is flipped"
        );
        assert_eq!(
            commands[1].autostart(),
            &Some(true),
            "the already-true sibling is untouched"
        );
    }

    #[test]
    fn t_persists_autostart_for_the_selected_identical_row() {
        // Two fully identical specs: only the row position tells them apart, so
        // toggling the second row must edit the second spec, or a reload would
        // move the user's choice onto the first row.
        let dup = || {
            ProcessSpec::builder()
                .name(ProcessName::try_new("dup").unwrap())
                .command(Some(CommandLine::try_new("true").unwrap()))
                .autostart(Some(false))
                .build()
        };
        let config = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![dup(), dup()])
            .build();
        let recorder = Recorder::default();
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(config.to_processes())
            .build();
        let registry = Box::new(FakeRegistry {
            projects: vec![],
            workspace: config,
            recorder: recorder.clone(),
        });
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            registry,
            PathBuf::from("/here/muster.yml"),
        );
        app.start();

        press(&mut app, KeyCode::Char('j')); // select the second identical row
        assert_eq!(*app.workspace.selected_index(), 1);
        press(&mut app, KeyCode::Char('t'));

        let workspaces = recorder.workspaces.borrow();
        let commands = workspaces[0].1.commands();
        assert_eq!(
            commands[0].autostart(),
            &Some(false),
            "the first identical spec is left untouched"
        );
        assert_eq!(
            commands[1].autostart(),
            &Some(true),
            "the second spec, matching the selected row, is the one flipped"
        );
    }

    #[test]
    fn t_on_an_untracked_process_does_not_report_persistence() {
        // A live process whose identity matches no spec (its config entry was
        // removed): autostart cannot be recorded, so the live flag must not move.
        let orphan = Process::builder()
            .id(PaneId::new(PANE))
            .name(ProcessName::try_new("ghost").unwrap())
            .kind(ProcessKind::Terminal)
            .command(Some(CommandLine::try_new("true").unwrap()))
            .autostart(true)
            .build();
        let recorder = Recorder::default();
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder().processes(vec![orphan]).build();
        let registry = Box::new(FakeRegistry {
            projects: vec![],
            workspace: empty_workspace_config(), // the config lists nothing
            recorder: recorder.clone(),
        });
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            registry,
            PathBuf::from("/here/muster.yml"),
        );

        press(&mut app, KeyCode::Char('t'));

        assert!(
            *app.workspace.selected_process().unwrap().autostart(),
            "a process with no config entry keeps its live flag unchanged"
        );
        assert!(
            app.notice.is_some(),
            "the user is told the toggle was not persisted"
        );
    }

    #[test]
    fn a_failed_autostart_write_leaves_the_live_flag_unchanged() {
        let config = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![
                ProcessSpec::builder()
                    .name(ProcessName::try_new("cmd").unwrap())
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .build(),
            ])
            .build();
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(config.to_processes())
            .build();
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            Box::new(BrokenRegistry {
                saved: Rc::new(RefCell::new(false)),
            }),
            PathBuf::from("/here/muster.yml"),
        );
        app.start();

        assert!(!*app.workspace.selected_process().unwrap().autostart());
        press(&mut app, KeyCode::Char('t'));

        assert!(
            !*app.workspace.selected_process().unwrap().autostart(),
            "a failed write does not flip the live autostart flag"
        );
        assert!(app.notice.is_some(), "the failure is surfaced as a notice");
    }

    #[test]
    fn the_help_overlay_opens_and_any_key_dismisses_it() {
        let mut app = app_with(RestartPolicy::Never);

        press(&mut app, KeyCode::Char('?'));
        assert!(help_open(&app), "? opens the keymap overlay");

        press(&mut app, KeyCode::Char('q'));
        assert!(!help_open(&app), "any key closes the overlay");
        assert!(
            app.is_running(),
            "the dismissing key is swallowed, not acted on"
        );
    }

    /// An app as `flow_app`, discarding the recorder.
    fn switcher_app(projects: Vec<Project>, target: WorkspaceConfig, current: &str) -> App {
        flow_app(projects, target, current).0
    }

    /// Sends a single key to the app.
    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    /// Types each character of `text` into the app.
    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    #[test]
    fn saving_the_current_workspace_registers_it() {
        let (mut app, recorder) = flow_app(vec![], empty_workspace_config(), "/here/muster.yml");
        app.open_switcher();
        press(&mut app, KeyCode::Char('s'));
        type_text(&mut app, "My Setup");
        press(&mut app, KeyCode::Enter);

        assert!(app.form().is_none(), "the form closes on success");
        let saved = recorder
            .projects
            .borrow()
            .clone()
            .expect("the registry was saved");
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].name().as_ref(), "My Setup");
        assert_eq!(saved[0].config(), &PathBuf::from("/here/muster.yml"));
    }

    #[test]
    fn canceling_a_switcher_form_restores_the_switcher() {
        let (mut app, _) = flow_app(vec![], empty_workspace_config(), "/here/muster.yml");
        app.open_switcher();
        press(&mut app, KeyCode::Char('n'));
        assert!(matches!(&app.overlay, Some(Overlay::Form(_))));

        press(&mut app, KeyCode::Esc);

        assert!(matches!(&app.overlay, Some(Overlay::Switcher(_))));
    }

    #[test]
    fn creating_a_new_project_writes_a_starter_config_and_registers_it() {
        let (mut app, recorder) = flow_app(vec![], empty_workspace_config(), "/here/muster.yml");
        app.open_switcher();
        app.new_project(&["fresh".to_string(), "/tmp/fresh-proj".to_string()]);

        let workspaces = recorder.workspaces.borrow();
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].0, PathBuf::from("/tmp/fresh-proj/muster.yml"));
        assert_eq!(
            workspaces[0].1.terminals().len(),
            1,
            "the starter config ships one terminal"
        );
        drop(workspaces);
        let saved = recorder
            .projects
            .borrow()
            .clone()
            .expect("the project was registered");
        assert_eq!(saved[0].name().as_ref(), "fresh");
        assert_eq!(
            saved[0].config(),
            &PathBuf::from("/tmp/fresh-proj/muster.yml")
        );
    }

    #[test]
    fn adding_a_process_writes_it_to_the_config() {
        let (mut app, recorder) =
            flow_app(vec![], one_agent_config("existing"), "/here/muster.yml");
        app.add_process(&[
            "terminal".to_string(),
            "logs".to_string(),
            "tail -f log".to_string(),
        ]);

        let workspaces = recorder.workspaces.borrow();
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].0, PathBuf::from("/here/muster.yml"));
        let config = &workspaces[0].1;
        assert_eq!(config.agents().len(), 1, "the existing agent is kept");
        assert_eq!(config.terminals().len(), 1);
        assert_eq!(config.terminals()[0].name().as_ref(), "logs");
    }

    #[test]
    fn removing_a_project_persists_the_shorter_list() {
        let projects = vec![
            project("alpha", "/a/muster.yml"),
            project("beta", "/b/muster.yml"),
        ];
        let (mut app, recorder) = flow_app(projects, empty_workspace_config(), "/a/muster.yml");
        app.open_switcher();
        press(&mut app, KeyCode::Char('d'));

        let saved = recorder
            .projects
            .borrow()
            .clone()
            .expect("the registry was saved");
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].name().as_ref(), "beta");
    }

    /// A registry whose reads fail, recording whether a save was attempted.
    struct BrokenRegistry {
        saved: Rc<RefCell<bool>>,
    }

    impl ProjectRegistry for BrokenRegistry {
        fn projects(&self) -> Result<Vec<Project>, ConfigError> {
            Err(ConfigError::Parse(
                serde_yaml_ng::from_str::<WorkspaceConfig>("42").unwrap_err(),
            ))
        }

        fn workspace(&self, _config_path: &Path) -> Result<WorkspaceConfig, ConfigError> {
            Err(ConfigError::Parse(
                serde_yaml_ng::from_str::<WorkspaceConfig>("42").unwrap_err(),
            ))
        }

        fn workspace_exists(&self, _config_path: &Path) -> bool {
            false
        }

        fn save(&self, _projects: &[Project]) -> Result<(), ConfigError> {
            *self.saved.borrow_mut() = true;
            Ok(())
        }

        fn save_workspace(
            &self,
            _config_path: &Path,
            _config: &WorkspaceConfig,
        ) -> Result<(), ConfigError> {
            Ok(())
        }
    }

    #[test]
    fn creating_a_project_asks_before_overwriting_then_overwrites() {
        let (mut app, recorder) = flow_app(vec![], empty_workspace_config(), "/here/muster.yml");
        // Simulate an existing config already in the target folder.
        recorder
            .workspaces
            .borrow_mut()
            .push((PathBuf::from("/taken/muster.yml"), empty_workspace_config()));
        app.open_switcher();
        app.open_new_project_form();
        app.new_project(&["proj".to_string(), "/taken".to_string()]);

        // A confirmation is shown; nothing is written yet.
        assert!(
            confirmation_open(&app),
            "an overwrite asks for confirmation"
        );
        assert_eq!(
            recorder.workspaces.borrow().len(),
            1,
            "nothing is written before confirming"
        );
        assert!(
            recorder.projects.borrow().is_none(),
            "nothing is registered before confirming"
        );

        // Confirming overwrites the config and registers the project.
        press(&mut app, KeyCode::Enter);
        assert!(!confirmation_open(&app), "confirming closes the dialog");
        assert_eq!(
            recorder.projects.borrow().as_ref().unwrap()[0]
                .name()
                .as_ref(),
            "proj"
        );
        assert!(
            recorder
                .workspaces
                .borrow()
                .iter()
                .any(|(path, _)| path == &PathBuf::from("/taken/muster.yml")),
            "the config is written on confirm"
        );
    }

    #[test]
    fn declining_an_overwrite_writes_nothing() {
        let (mut app, recorder) = flow_app(vec![], empty_workspace_config(), "/here/muster.yml");
        recorder
            .workspaces
            .borrow_mut()
            .push((PathBuf::from("/taken/muster.yml"), empty_workspace_config()));
        app.open_switcher();
        app.open_new_project_form();
        app.new_project(&["proj".to_string(), "/taken".to_string()]);

        press(&mut app, KeyCode::Char('n'));

        assert!(!confirmation_open(&app), "declining closes the dialog");
        assert!(
            app.form().is_some(),
            "declining restores the populated form"
        );
        assert!(
            recorder.projects.borrow().is_none(),
            "nothing is registered"
        );
        assert_eq!(
            recorder.workspaces.borrow().len(),
            1,
            "only the pre-existing config remains"
        );
    }

    #[test]
    fn a_failed_confirmed_overwrite_keeps_the_form_for_retry() {
        let recorder = Recorder::default();
        recorder
            .workspaces
            .borrow_mut()
            .push((PathBuf::from("/taken/muster.yml"), empty_workspace_config()));
        let mut app = controlled_app(ControlledRegistry {
            workspace: empty_workspace_config(),
            fail_save: false,
            fail_save_workspace: true,
            recorder,
        });
        app.open_switcher();
        app.open_new_project_form();
        app.new_project(&["proj".to_string(), "/taken".to_string()]);
        assert!(confirmation_open(&app), "an overwrite asks first");

        // Confirm; the workspace write then fails.
        press(&mut app, KeyCode::Enter);

        assert!(!confirmation_open(&app), "the confirmation closes");
        let form = app
            .form()
            .expect("the form is kept so the user can retry without refilling it");
        assert!(form.error.is_some(), "the failure is shown on the form");
    }

    #[test]
    fn dropdown_navigation_keeps_the_highlight() {
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(PANE))
                    .name(ProcessName::try_new("p").unwrap())
                    .kind(ProcessKind::Terminal)
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .restart(RestartPolicy::Never)
                    .build(),
            ])
            .build();
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(CannedCompleter(vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
            ])),
            empty_registry(),
            PathBuf::from("/here/muster.yml"),
        );
        app.start();

        app.open_new_project_form();
        press(&mut app, KeyCode::Tab); // focus the folder field so candidates populate
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);

        let folder = &app.form().unwrap().form.fields()[1];
        assert_eq!(
            folder.highlighted(),
            2,
            "arrows move the highlight through App, not reset it to the first"
        );
    }

    #[test]
    fn accepting_a_completion_does_not_reopen_the_dropdown() {
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(PANE))
                    .name(ProcessName::try_new("p").unwrap())
                    .kind(ProcessKind::Terminal)
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .restart(RestartPolicy::Never)
                    .build(),
            ])
            .build();
        // This completer offers candidates for any value, so a refresh after
        // acceptance would immediately repopulate the dropdown and trap the user.
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(CannedCompleter(vec![
                "alpha".to_string(),
                "beta".to_string(),
            ])),
            empty_registry(),
            PathBuf::from("/here/muster.yml"),
        );
        app.start();

        app.open_new_project_form();
        type_text(&mut app, "proj"); // name field
        press(&mut app, KeyCode::Tab); // focus folder -> candidates populate
        press(&mut app, KeyCode::Down); // highlight "beta"
        press(&mut app, KeyCode::Enter); // accept it

        let folder = &app.form().unwrap().form.fields()[1];
        assert_eq!(
            folder.value(),
            "beta",
            "the accepted candidate fills the field"
        );
        assert!(
            folder.candidates().is_empty(),
            "the dropdown stays closed after accepting, despite the completer offering more"
        );

        // With the dropdown closed, Enter now submits and creates the project.
        press(&mut app, KeyCode::Enter);
        assert!(
            app.form().is_none(),
            "Enter submits instead of accepting a child"
        );
    }

    #[test]
    fn completion_worker_delivers_candidates_off_the_event_loop() {
        let (sender, receiver) = bounded(16);
        let workspace = Workspace::builder().processes(vec![]).build();
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(CannedCompleter(vec![
                "alpha".to_string(),
                "beta".to_string(),
            ])),
            empty_registry(),
            PathBuf::from("/here/muster.yml"),
        );
        app.spawn_completion_worker();

        app.open_new_project_form();
        press(&mut app, KeyCode::Tab); // focus the folder field -> dispatch a request

        // The worker computes on another thread; block for its reply and apply it
        // exactly as the runtime loop would, proving completion is off the loop.
        let (generation, candidates) = loop {
            match receiver.recv_timeout(Duration::from_secs(1)).unwrap() {
                RuntimeEvent::Completions {
                    generation,
                    candidates,
                } => break (generation, candidates),
                _ => continue,
            }
        };
        app.handle_completions(generation, candidates);

        let folder = &app.form().unwrap().form.fields()[1];
        assert_eq!(
            folder.candidates().to_vec(),
            vec!["alpha".to_string(), "beta".to_string()],
            "candidates arrive through the event channel"
        );
    }

    #[test]
    fn a_superseded_completion_result_is_discarded() {
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder().processes(vec![]).build();
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(CannedCompleter(vec!["fresh".to_string()])),
            empty_registry(),
            PathBuf::from("/here/muster.yml"),
        );
        app.spawn_completion_worker();

        app.open_new_project_form();
        press(&mut app, KeyCode::Tab); // generation 1
        type_text(&mut app, "x"); // an edit bumps to generation 2

        // A late reply tagged with the earlier generation must not repopulate.
        app.handle_completions(CompletionGeneration::new(1), vec!["stale".to_string()]);

        let folder = &app.form().unwrap().form.fields()[1];
        assert!(
            !folder
                .candidates()
                .iter()
                .any(|candidate| candidate == "stale"),
            "a result from a superseded generation is ignored"
        );
    }

    #[test]
    fn editing_a_path_drops_stale_candidates_before_the_reply() {
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder().processes(vec![]).build();
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(CannedCompleter(vec![
                "alpha".to_string(),
                "beta".to_string(),
            ])),
            empty_registry(),
            PathBuf::from("/here/muster.yml"),
        );
        app.spawn_completion_worker();

        app.open_new_project_form();
        type_text(&mut app, "proj"); // name field
        press(&mut app, KeyCode::Tab); // focus folder -> generation 1
        // Simulate the worker's generation-1 reply populating the dropdown.
        app.handle_completions(CompletionGeneration::new(1), vec![
            "alpha".to_string(),
            "beta".to_string(),
        ]);
        assert_eq!(
            app.form().unwrap().form.fields()[1].candidates().len(),
            2,
            "the dropdown is populated before the edit"
        );

        // Editing the path must immediately drop the now-stale candidates rather
        // than leave them acceptable against a value they no longer match.
        type_text(&mut app, "z");
        let edited = app.form().unwrap().form.fields()[1].value();
        assert!(
            app.form().unwrap().form.fields()[1].candidates().is_empty(),
            "the stale dropdown closes the instant the value changes"
        );
        assert!(edited.ends_with('z'), "the typed character survives");

        // With the dropdown closed, Enter submits the edited value instead of
        // replacing it with a stale candidate.
        press(&mut app, KeyCode::Enter);
        assert!(
            app.form().is_none(),
            "Enter submits rather than accepting a stale candidate"
        );
    }

    #[test]
    fn a_failed_registry_read_is_not_overwritten() {
        let saved = Rc::new(RefCell::new(false));
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(PANE))
                    .name(ProcessName::try_new("p").unwrap())
                    .kind(ProcessKind::Terminal)
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .restart(RestartPolicy::Never)
                    .build(),
            ])
            .build();
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            Box::new(BrokenRegistry {
                saved: saved.clone(),
            }),
            PathBuf::from("/here/muster.yml"),
        );
        app.start();

        app.open_save_project_form();
        app.save_current_project(&["proj".to_string()]);

        assert!(
            !*saved.borrow(),
            "an unreadable registry is never overwritten"
        );
        assert!(
            app.form().unwrap().error.is_some(),
            "the read failure is reported"
        );
    }

    /// A registry with switchable save failures, recording successful writes.
    struct ControlledRegistry {
        workspace: WorkspaceConfig,
        fail_save: bool,
        fail_save_workspace: bool,
        recorder: Recorder,
    }

    fn write_error() -> ConfigError {
        ConfigError::Write {
            path: PathBuf::from("/denied/muster.yml"),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        }
    }

    impl ProjectRegistry for ControlledRegistry {
        fn projects(&self) -> Result<Vec<Project>, ConfigError> {
            Ok(self.recorder.projects.borrow().clone().unwrap_or_default())
        }

        fn workspace(&self, _config_path: &Path) -> Result<WorkspaceConfig, ConfigError> {
            Ok(self.workspace.clone())
        }

        fn workspace_exists(&self, config_path: &Path) -> bool {
            self.recorder
                .workspaces
                .borrow()
                .iter()
                .any(|(path, _)| path == config_path)
        }

        fn save(&self, projects: &[Project]) -> Result<(), ConfigError> {
            if self.fail_save {
                return Err(write_error());
            }
            *self.recorder.projects.borrow_mut() = Some(projects.to_vec());
            Ok(())
        }

        fn save_workspace(
            &self,
            config_path: &Path,
            config: &WorkspaceConfig,
        ) -> Result<(), ConfigError> {
            if self.fail_save_workspace {
                return Err(write_error());
            }
            self.recorder
                .workspaces
                .borrow_mut()
                .push((config_path.to_path_buf(), config.clone()));
            Ok(())
        }
    }

    /// An app started on a single "p" process, backed by `registry`.
    fn controlled_app(registry: ControlledRegistry) -> App {
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(PANE))
                    .name(ProcessName::try_new("p").unwrap())
                    .kind(ProcessKind::Terminal)
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .restart(RestartPolicy::Never)
                    .build(),
            ])
            .build();
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            Box::new(registry),
            PathBuf::from("/here/muster.yml"),
        );
        app.start();
        app
    }

    #[test]
    fn a_failed_workspace_write_still_registers_a_recoverable_project() {
        let recorder = Recorder::default();
        let mut app = controlled_app(ControlledRegistry {
            workspace: empty_workspace_config(),
            fail_save: false,
            fail_save_workspace: true,
            recorder: recorder.clone(),
        });

        app.open_new_project_form();
        app.new_project(&["fresh".to_string(), "/tmp/fresh".to_string()]);

        let saved = recorder
            .projects
            .borrow()
            .clone()
            .expect("the project is registered before the file is written");
        assert_eq!(saved[0].name().as_ref(), "fresh");
        assert!(
            recorder.workspaces.borrow().is_empty(),
            "no config file is written when the write fails"
        );
        assert!(
            app.form().unwrap().error.is_some(),
            "the write failure is reported"
        );
    }

    #[test]
    fn a_failed_add_process_write_is_reported() {
        let mut app = controlled_app(ControlledRegistry {
            workspace: empty_workspace_config(),
            fail_save: false,
            fail_save_workspace: true,
            recorder: Recorder::default(),
        });

        app.open_add_process_form();
        app.add_process(&[
            "terminal".to_string(),
            "logs".to_string(),
            "true".to_string(),
        ]);

        assert!(
            app.form().unwrap().error.is_some(),
            "the write failure is reported, not silently swallowed"
        );
    }

    #[test]
    fn a_failed_removal_preserves_the_list_and_reports() {
        let recorder = Recorder::default();
        *recorder.projects.borrow_mut() = Some(vec![
            project("alpha", "/a/muster.yml"),
            project("beta", "/b/muster.yml"),
        ]);
        let mut app = controlled_app(ControlledRegistry {
            workspace: empty_workspace_config(),
            fail_save: true,
            fail_save_workspace: false,
            recorder: recorder.clone(),
        });

        app.open_switcher();
        app.remove_selected_project();

        assert!(
            app.switcher().unwrap().error.is_some(),
            "the removal failure is reported in the switcher"
        );
        assert_eq!(
            recorder.projects.borrow().as_ref().unwrap().len(),
            2,
            "the project list is preserved when the write fails"
        );
    }

    #[test]
    fn opening_the_switcher_preselects_the_current_project() {
        let projects = vec![
            project("alpha", "/a/muster.yml"),
            project("beta", "/b/muster.yml"),
        ];
        let mut app = switcher_app(projects, empty_workspace_config(), "/b/muster.yml");

        app.open_switcher();

        let switcher = app.switcher().expect("switcher is open");
        assert_eq!(switcher.projects.len(), 2);
        assert_eq!(switcher.selected, 1, "the current project is preselected");
    }

    #[test]
    fn a_relative_current_config_matches_an_absolute_registry_entry() {
        // `cargo test` runs from the crate root, so "Cargo.toml" and its absolute
        // form name the same file once normalized.
        let absolute = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let projects = vec![
            project("other", "/nonexistent/muster.yml"),
            Project::builder()
                .name(ProjectName::try_new("here").unwrap())
                .config(absolute)
                .build(),
        ];
        let mut app = switcher_app(projects, empty_workspace_config(), "Cargo.toml");

        app.open_switcher();

        let switcher = app.switcher().expect("switcher is open");
        assert_eq!(
            switcher.selected, 1,
            "the current project is recognized despite differing path forms"
        );
    }

    fn one_agent_config(name: &str) -> WorkspaceConfig {
        WorkspaceConfig::builder()
            .agents(vec![
                ProcessSpec::builder()
                    .name(ProcessName::try_new(name).unwrap())
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .build(),
            ])
            .terminals(vec![])
            .commands(vec![])
            .build()
    }

    #[test]
    fn jumping_by_number_switches_after_the_old_process_exits() {
        let projects = vec![
            project("alpha", "/a/muster.yml"),
            project("beta", "/b/muster.yml"),
        ];
        let mut app = switcher_app(projects, one_agent_config("switched"), "/a/muster.yml");
        assert_eq!(app.workspace.processes()[0].name().as_ref(), "p");

        app.open_switcher();
        app.handle_switcher_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));

        // The overlay closes, but the load waits for the old child to exit so the
        // new project never contends for its resources.
        assert!(
            app.switcher().is_none(),
            "a requested switch closes the overlay"
        );
        assert_eq!(
            app.workspace.processes()[0].name().as_ref(),
            "p",
            "the new project does not start until the old child exits"
        );

        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Exited(ExitOutcome::Succeeded),
        );

        assert_eq!(app.workspace.processes().len(), 1);
        assert_eq!(app.workspace.processes()[0].name().as_ref(), "switched");
        assert_eq!(app.current_config, Some(PathBuf::from("/b/muster.yml")));
        assert!(
            app.panes.contains_key(&PaneId::new(0)),
            "the switched-in process spawns once the old one exits"
        );
    }

    #[test]
    fn switching_with_no_live_process_loads_immediately() {
        let projects = vec![
            project("alpha", "/a/muster.yml"),
            project("beta", "/b/muster.yml"),
        ];
        let mut app = switcher_app(projects, one_agent_config("switched"), "/a/muster.yml");
        // The current process has already exited: nothing is running.
        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Exited(ExitOutcome::Succeeded),
        );
        assert!(app.panes.get(&PaneId::new(PANE)).unwrap().handle.is_none());

        app.open_switcher();
        app.handle_switcher_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));

        assert_eq!(
            app.workspace.processes()[0].name().as_ref(),
            "switched",
            "with no live children the switch happens at once"
        );
    }

    #[test]
    fn escape_closes_the_switcher_without_switching() {
        let projects = vec![project("alpha", "/a/muster.yml")];
        let mut app = switcher_app(projects, empty_workspace_config(), "/a/muster.yml");

        app.open_switcher();
        app.handle_switcher_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(app.switcher().is_none());
        assert_eq!(
            app.workspace.processes()[0].name().as_ref(),
            "p",
            "the workspace is unchanged"
        );
    }

    #[test]
    fn a_failed_switch_surfaces_an_error_and_stays_open() {
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(PANE))
                    .name(ProcessName::try_new("p").unwrap())
                    .kind(ProcessKind::Terminal)
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .restart(RestartPolicy::Never)
                    .build(),
            ])
            .build();
        let registry = Box::new(FailingRegistry {
            projects: vec![project("alpha", "/a/muster.yml")],
        });
        let mut app = App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            registry,
            PathBuf::from("/a/muster.yml"),
        );
        app.start();

        app.open_switcher();
        app.handle_switcher_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let switcher = app.switcher().expect("stays open after a failed switch");
        assert!(switcher.error.is_some());
        assert_eq!(
            app.workspace.processes()[0].name().as_ref(),
            "p",
            "the workspace is unchanged on failure"
        );
    }
}

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::OsString,
    mem,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, unbounded};
use crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};
use fake::{Fake, faker::name::en::FirstName};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Position, Rect},
    style::Style,
};
use vt100::{MouseProtocolMode, Parser, Screen};

use super::{
    activity::ActivityTracker,
    activity_frame::{ACTIVITY_FRAME_INTERVAL, ActivityFrame},
    completion_generation::CompletionGeneration,
    event::{ChannelOutputSink, RuntimeEvent},
    form::{Field, Form, FormOutcome},
    input,
    pointer_shape::PointerShape,
    selection::{
        self, Autoscroll, AutoscrollDirection, BufferCell, GridCell, ScrollMetrics, Selection,
    },
    shutdown_generation::ShutdownGeneration,
    signal::{Signal, SignalReader},
    spawn_generation::SpawnGeneration,
    widget::{
        agent_picker::{self, AgentPickerItem},
        confirm, empty_state, form, help, sidebar, status_bar,
        status_bar::{NoticeTone, StatusContext},
        switcher, terminal_pane, theme,
    },
};
mod process_controller;
mod project_controller;
mod session_controller;

use crate::{
    adapter::path,
    application::{
        ExitDecision, ProcessLifecycle, ProcessSpecMatcher, Reconciliation, SessionRestorer,
        Workspace,
    },
    constants::{APP_NAME, MUSTER_AGENT_SESSION_ENV, MUSTER_AGENT_SESSION_STATE_FILE_ENV},
    domain::{
        agent_session::{AgentSession, AgentSessionId, AgentSessionState, NativeSessionId},
        config::{ConfigError, ProcessSpec, WorkspaceConfig},
        notification::{Notification, NotificationId, NotificationScope},
        port::{
            AgentSessionStore, ConfigWatcher, Notifier, PathCompleter, ProcessHandle,
            ProcessRunner, ProjectRegistry, SettingsStore,
        },
        process::{
            ActivityState, AgentProtocol, AgentTool, ExitIntent, Process, ProcessKind,
            ProcessOrigin, ProcessState, StopPolicy,
        },
        project::Project,
        pty::{ExitOutcome, ProcessOutput, PtySize, SpawnRequest},
        settings::Settings,
        value::{Cols, CommandLine, PaneId, ProcessName, ProjectName, Rows},
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
/// Lines the pane view moves per mouse-wheel notch.
const WHEEL_SCROLL_LINES: usize = 3;
/// Delay between edge-drag autoscroll steps (herdr's cadence).
const SELECTION_AUTOSCROLL_INTERVAL: Duration = Duration::from_millis(30);
/// Window within which a second pane click counts as a double-click.
const PANE_DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(350);
/// How long a double-click copy keeps its confirming highlight.
const PANE_COPY_HIGHLIGHT_DURATION: Duration = Duration::from_millis(500);
/// Notice shown for a bare bell notification that carried no text of its own.
const AWAITING_INPUT_NOTICE: &str = "awaiting input";
/// Minimum gap between bell notifications from one pane, absorbing a burst (e.g.
/// shell tab-completion) into a single alert.
const BELL_THROTTLE: Duration = Duration::from_secs(3);
/// Quiet interval before a background terminal bell becomes a desktop alert.
const BELL_ESCALATION_DELAY: Duration = Duration::from_secs(2);
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
/// Shown when a graceful shutdown deadline expires and Muster force-kills the
/// selected command's process group.
const FORCE_STOP_NOTICE: &str = "graceful shutdown timed out; force-killed";
/// Shown when graceful signal delivery fails and Muster uses an immediate kill.
const GRACEFUL_STOP_FALLBACK_NOTICE: &str = "graceful shutdown failed; force-killed";
/// Shown when Muster cannot deliver either the requested signal or a hard kill.
const STOP_DELIVERY_FAILED_NOTICE: &str = "could not stop the process";
/// Shown when a session reopen is unavailable until project teardown completes.
const PROJECT_SWITCH_IN_PROGRESS: &str = "project switch in progress; reopen after it completes";
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
/// Title of the advanced agent-session form.
const ADD_AGENT_TITLE: &str = "New agent session";
/// Label of a process-kind field.
const KIND_FIELD: &str = "Kind";
/// Label of an agent-tool field.
const TOOL_FIELD: &str = "Tool";
/// Label of an optional agent-session display name.
const SESSION_NAME_FIELD: &str = "Name (optional)";
/// Label of an optional preset command override.
const AGENT_COMMAND_FIELD: &str = "Command override";
/// Label of an optional custom provider resume command.
const AGENT_RESUME_FIELD: &str = "Resume command (optional)";
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
/// Shown when a custom agent session has no launch command.
const AGENT_COMMAND_REQUIRED: &str = "a custom agent needs a command";
/// Shown when a custom resume placeholder cannot be expanded safely.
const AGENT_RESUME_TEMPLATE_INVALID: &str =
    "resume placeholder must be an unquoted standalone shell word";
/// Shown when a shell composition needs an explicit provider resume command.
const AGENT_COMPOUND_RESUME_REQUIRED: &str = "shell compositions need an explicit resume command";
/// Shown when autostart is requested for a runtime agent session.
const SESSION_AUTOSTART_UNAVAILABLE: &str =
    "open agent sessions restore automatically; use d to close one";
/// Shown when close is requested for a configured process.
const CONFIGURED_AGENT_CLOSE_UNAVAILABLE: &str = "configured agents stay pinned in muster.yml";
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
/// Title of the agent-session close confirmation.
const CLOSE_AGENT_TITLE: &str = "Close agent?";
/// Accept-action verb for the agent-session close confirmation.
const CLOSE_AGENT_VERB: &str = "close";
/// Maximum resumable history rows shown above provider presets.
const RECENT_AGENT_LIMIT: usize = 5;
/// Attempts to avoid a generated display-name collision in the active workspace.
const GENERATED_NAME_ATTEMPTS: usize = 8;
/// Shown when durable agent-session state cannot be loaded or written.
const AGENT_SESSION_STORE_ERROR: &str = "could not update agent session history";
/// Shown when a closed session has no provider identity to resume.
const AGENT_SESSION_NOT_RESUMABLE: &str =
    "the provider session ID was not captured; run `muster hooks setup`";
/// Shown when there is no closed resumable session in this workspace.
const NO_RECENT_AGENT_SESSION: &str = "no closed agent session to reopen";
/// Placeholder identity used only to validate provider resume command shape.
const RESUME_VALIDATION_ID: &str = "muster-resume-validation";

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

/// How a persisted agent session should enter the active workspace.
enum AgentSessionActivation {
    /// Show a closable row without starting a new provider conversation.
    Stopped,
    /// Start the provider conversation but keep keyboard focus in the sidebar.
    StartDetached(CommandLine),
    /// Start and attach the provider conversation immediately.
    StartAttached(CommandLine),
}

impl AgentSessionActivation {
    /// Returns the command to spawn when this activation starts a child.
    fn command(&self) -> Option<&CommandLine> {
        match self {
            Self::Stopped => None,
            Self::StartDetached(command) | Self::StartAttached(command) => Some(command),
        }
    }

    /// Whether successful activation should attach keyboard focus to the pane.
    fn should_attach(&self) -> bool {
        matches!(self, Self::StartAttached(_))
    }
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
    /// Delayed desktop alert for a background non-agent pane that rang its bell.
    pending_bell_notification: Option<Instant>,
    handle: Option<Box<dyn ProcessHandle>>,
    started_at: Instant,
    exit_intent: ExitIntent,
    shutdown_generation: ShutdownGeneration,
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

/// Open state of the quick agent launcher.
struct AgentPicker {
    items: Vec<AgentPickerItem>,
    selected: usize,
    error: Option<String>,
}

impl Switcher {
    /// Loads `project`'s processes for a preview, or returns an empty list when
    /// there is no project or its config cannot be read.
    fn preview(
        registry: &dyn ProjectRegistry,
        project: Option<&Project>,
    ) -> Vec<(ProcessKind, String)> {
        project
            .and_then(|project| path::registered_config_path(project).ok())
            .and_then(|config| registry.workspace(&config).ok())
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
    AgentPicker(AgentPicker),
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
    ConfirmSessionClose {
        message: String,
        pane: PaneId,
    },
    Help,
}

impl Overlay {
    /// Returns the switcher in this overlay stack, if present.
    fn switcher(&self) -> Option<&Switcher> {
        match self {
            Self::Switcher(switcher) => Some(switcher),
            Self::Form(form) | Self::ConfirmOverwrite { form, .. } => form.switcher.as_ref(),
            Self::AgentPicker(_)
            | Self::ConfirmRemoval { .. }
            | Self::ConfirmSessionClose { .. }
            | Self::Help => None,
        }
    }

    /// Returns the switcher in this overlay stack mutably, if present.
    fn switcher_mut(&mut self) -> Option<&mut Switcher> {
        match self {
            Self::Switcher(switcher) => Some(switcher),
            Self::Form(form) | Self::ConfirmOverwrite { form, .. } => form.switcher.as_mut(),
            Self::AgentPicker(_)
            | Self::ConfirmRemoval { .. }
            | Self::ConfirmSessionClose { .. }
            | Self::Help => None,
        }
    }

    /// Returns the form in this overlay stack, if present.
    fn form(&self) -> Option<&FormModal> {
        match self {
            Self::Form(form) | Self::ConfirmOverwrite { form, .. } => Some(&form.modal),
            Self::Switcher(_)
            | Self::AgentPicker(_)
            | Self::ConfirmRemoval { .. }
            | Self::ConfirmSessionClose { .. }
            | Self::Help => None,
        }
    }

    /// Returns the form in this overlay stack mutably, if present.
    fn form_mut(&mut self) -> Option<&mut FormModal> {
        match self {
            Self::Form(form) | Self::ConfirmOverwrite { form, .. } => Some(&mut form.modal),
            Self::Switcher(_)
            | Self::AgentPicker(_)
            | Self::ConfirmRemoval { .. }
            | Self::ConfirmSessionClose { .. }
            | Self::Help => None,
        }
    }

    /// Renders this overlay and any explicit background it retains.
    fn render(&self, frame: &mut Frame) {
        let area = frame.area();
        match self {
            Self::Switcher(switcher) => Self::render_switcher(frame, area, switcher),
            Self::AgentPicker(picker) => agent_picker::render(
                frame,
                area,
                &picker.items,
                picker.selected,
                picker.error.as_deref(),
            ),
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
            Self::ConfirmSessionClose { message, .. } => {
                confirm::render(frame, area, CLOSE_AGENT_TITLE, message, CLOSE_AGENT_VERB)
            },
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
    /// Choose which kind of process to add.
    ChooseProcessKind,
    /// Launch a runtime-managed coding-agent session.
    LaunchAgentSession,
    /// Persist a terminal or command in the current workspace.
    AddConfiguredProcess(ProcessKind),
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
    /// Current glyph in the working-agent spinner cycle.
    activity_frame: ActivityFrame,
    /// Next time a visible working-agent spinner should advance.
    activity_frame_deadline: Instant,
    frame_area: Rect,
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
    /// Reopen requests waiting for the prior pane of the same session to finish
    /// retiring, keyed by that pane's identity.
    pending_session_reopens: HashMap<PaneId, AgentSessionId>,
    /// Out-of-band notification delivery (desktop), injected by the composition
    /// root via [`Self::set_notifier`]. `None` until then: notifications stay
    /// in-app, and the App itself never names a concrete notifier adapter.
    notifier: Option<Box<dyn Notifier>>,
    /// Cross-workspace settings and their load result.
    settings: SettingsState,
    /// Durable provider session identity and history.
    agent_session_store: Option<Box<dyn AgentSessionStore>>,
    /// A transient one-line message shown in the status bar until the next key,
    /// used for failures that have no open overlay to report into.
    notice: Option<String>,
    /// The in-progress or lingering drag selection over a pane's buffer.
    selection: Option<Selection>,
    /// Active edge-drag autoscroll while a selection is held.
    selection_autoscroll: Option<Autoscroll>,
    /// Next tick of the edge-drag autoscroll.
    autoscroll_deadline: Option<Instant>,
    /// When a finalized word-copy highlight disappears.
    selection_clear_deadline: Option<Instant>,
    /// The last pane press, kept briefly to detect a double-click.
    last_pane_click: Option<PaneClick>,
    /// Viewport span of the selection, cached for the next immutable render.
    selection_view: Option<(GridCell, GridCell)>,
    /// Highlight style for selected cells, derived from the host terminal.
    selection_style: Style,
    /// Text a completed selection queued for the host clipboard; the runtime
    /// loop drains it because only the terminal guard writes to stdout.
    pending_clipboard: Option<String>,
    /// Pointer shape the host terminal currently shows for muster.
    pointer_shape: PointerShape,
    /// Pointer-shape change awaiting the runtime's terminal writer.
    pending_pointer_shape: Option<PointerShape>,
}

/// A pane press remembered briefly to recognize a double-click.
#[derive(Clone, Copy)]
struct PaneClick {
    pane: PaneId,
    cell: BufferCell,
    at: Instant,
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
        let current_config = path::absolutize(&current_config);
        Self {
            workspace,
            runner,
            events,
            panes: HashMap::new(),
            restart_attempts: HashMap::new(),
            generations: HashMap::new(),
            next_notification_scope: NotificationScope::new(0),
            activity_frame: ActivityFrame::initial(),
            activity_frame_deadline: Instant::now() + ACTIVITY_FRAME_INTERVAL,
            frame_area: area,
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
            pending_session_reopens: HashMap::new(),
            notifier: None,
            settings: SettingsState::Unloaded,
            agent_session_store: None,
            notice: None,
            selection: None,
            selection_autoscroll: None,
            autoscroll_deadline: None,
            selection_clear_deadline: None,
            last_pane_click: None,
            selection_view: None,
            selection_style: theme::selection_style(),
            pending_clipboard: None,
            pointer_shape: PointerShape::Default,
            pending_pointer_shape: None,
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

    /// Wires durable agent-session history. Loading stays lazy so project
    /// switches and hook-process updates are always observed fresh.
    pub fn set_agent_session_store(&mut self, store: Box<dyn AgentSessionStore>) {
        self.agent_session_store = Some(store);
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
            Some(
                Overlay::Switcher(_)
                | Overlay::AgentPicker(_)
                | Overlay::ConfirmRemoval { .. }
                | Overlay::ConfirmSessionClose { .. }
                | Overlay::Help,
            )
            | None => None,
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
        self.restore_open_agent_sessions();
        self.refresh_projects();
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
            CrosstermEvent::Mouse(mouse) => self.handle_mouse(mouse),
            CrosstermEvent::Resize(width, height) => self.resize(Rect::new(0, 0, width, height)),
            _ => {},
        }
    }

    /// Takes the text a completed selection queued for the host clipboard.
    pub fn take_pending_clipboard(&mut self) -> Option<String> {
        self.pending_clipboard.take()
    }

    /// Takes a pointer-shape change awaiting the terminal writer.
    pub fn take_pending_pointer_shape(&mut self) -> Option<PointerShape> {
        self.pending_pointer_shape.take()
    }

    /// Sets the selection highlight style derived from the host terminal.
    pub fn set_selection_style(&mut self, style: Style) {
        self.selection_style = style;
    }

    /// Routes pointer input the way herdr does: sidebar rows and panes focus
    /// on click, a child that requested mouse reports receives events
    /// directly, the wheel scrolls, and a left drag selects pane text.
    fn handle_mouse(&mut self, mouse: MouseEvent) {
        if self.overlay.is_some() {
            self.clear_selection();
            return;
        }
        let (sidebar_area, main_area, _) = areas(self.frame_area);
        self.refresh_pointer_shape(main_area, mouse);
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && sidebar_area.contains(Position::new(mouse.column, mouse.row))
        {
            self.clear_selection();
            self.handle_sidebar_press(sidebar_area, mouse);
            return;
        }
        let Some(pane) = self.selected_pane() else {
            return;
        };
        if self.pane_wants_mouse(pane) {
            self.clear_selection();
            if matches!(mouse.kind, MouseEventKind::Down(_))
                && main_area.contains(Position::new(mouse.column, mouse.row))
            {
                self.focus = Focus::Terminal;
            }
            self.forward_mouse_to_pane(pane, main_area, mouse);
            return;
        }
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.handle_left_press(pane, main_area, mouse);
            },
            MouseEventKind::Drag(MouseButton::Left) => {
                self.handle_left_drag(pane, main_area, mouse)
            },
            MouseEventKind::Up(MouseButton::Left) => self.handle_left_release(pane),
            MouseEventKind::ScrollUp => self.handle_wheel(pane, main_area, mouse, true),
            MouseEventKind::ScrollDown => self.handle_wheel(pane, main_area, mouse, false),
            _ => {},
        }
    }

    /// Focuses the sidebar on any click into it and, when the click lands on
    /// a row, selects that row the way keyboard navigation would.
    fn handle_sidebar_press(&mut self, sidebar_area: Rect, mouse: MouseEvent) {
        self.focus = Focus::Sidebar;
        let (active_label, other_projects, selection) = self.sidebar_context();
        let state = sidebar::SidebarState::builder()
            .workspace(&self.workspace)
            .activity_frame(self.activity_frame)
            .focused(true)
            .active_project(&active_label)
            .other_projects(&other_projects)
            .selection(selection)
            .build();
        let Some(clicked) =
            sidebar::selection_at(&state, sidebar_area, Position::new(mouse.column, mouse.row))
        else {
            return;
        };
        match clicked {
            sidebar::SidebarSelection::Process(index) => {
                self.project_cursor = None;
                self.workspace.select_at(index);
            },
            sidebar::SidebarSelection::Project(index) => {
                self.project_cursor = Some(index);
            },
        }
    }

    /// A left press in the pane: focuses the terminal, detects a double-click
    /// word copy, and otherwise anchors a new selection at the pressed cell.
    fn handle_left_press(&mut self, pane: PaneId, main_area: Rect, mouse: MouseEvent) {
        self.set_autoscroll(None);
        self.selection_clear_deadline = None;
        let cell = self
            .pane_grid(pane, main_area)
            .and_then(|grid| selection::cell_at(grid, mouse.column, mouse.row));
        let Some(cell) = cell else {
            self.clear_selection();
            return;
        };
        self.focus = Focus::Terminal;
        let Some(metrics) = self.pane_scroll_metrics(pane) else {
            return;
        };
        let pressed = metrics.buffer_cell(cell);
        let click = PaneClick {
            pane,
            cell: pressed,
            at: Instant::now(),
        };
        if self.take_double_click(click) && self.copy_word_at(pane, cell, pressed) {
            return;
        }
        self.selection = Some(Selection::anchored(pane, pressed));
    }

    /// Consumes a double-click candidate: true when this press matches the
    /// previous one closely enough (herdr's same-cell, 350 ms rule).
    fn take_double_click(&mut self, click: PaneClick) -> bool {
        let matched = self.last_pane_click.is_some_and(|last| {
            last.pane == click.pane
                && last.cell == click.cell
                && click.at.duration_since(last.at) <= PANE_DOUBLE_CLICK_WINDOW
        });
        self.last_pane_click = (!matched).then_some(click);
        matched
    }

    /// Copies the token under a double-click, leaving a short-lived highlight
    /// as confirmation (herdr's word copy). Returns whether text was copied.
    fn copy_word_at(&mut self, pane: PaneId, cell: GridCell, pressed: BufferCell) -> bool {
        let Some(target) = self.panes.get(&pane) else {
            return false;
        };
        let screen = target.parser.screen();
        let (_, columns) = screen.size();
        let row_text = screen.contents_between(cell.row(), 0, cell.row(), columns);
        let Some((start_column, end_column)) = selection::word_bounds(&row_text, cell.column())
        else {
            return false;
        };
        let text = screen.contents_between(
            cell.row(),
            start_column,
            cell.row(),
            end_column.saturating_add(1).min(columns),
        );
        if text.is_empty() {
            return false;
        }
        self.pending_clipboard = Some(text);
        let start = BufferCell::builder()
            .row(pressed.row())
            .column(start_column)
            .build();
        let end = BufferCell::builder()
            .row(pressed.row())
            .column(end_column)
            .build();
        self.selection = Some(Selection::word(pane, start, end));
        self.selection_clear_deadline = Some(Instant::now() + PANE_COPY_HIGHLIGHT_DURATION);
        true
    }

    /// Extends the held selection toward the pointer, driving herdr's edge
    /// autoscroll zones when the drag reaches or leaves the pane vertically.
    fn handle_left_drag(&mut self, pane: PaneId, main_area: Rect, mouse: MouseEvent) {
        self.last_pane_click = None;
        let holding = self
            .selection
            .as_ref()
            .is_some_and(|active| active.pane() == pane && active.is_in_progress());
        if !holding {
            return;
        }
        let Some(grid) = self.pane_grid(pane, main_area) else {
            return;
        };
        self.extend_selection_to(pane, grid, mouse.column, mouse.row);
        if !self.selection.as_ref().is_some_and(Selection::is_dragging) {
            self.set_autoscroll(None);
            return;
        }
        let top = grid.y;
        let bottom = grid.y + grid.height - 1;
        if mouse.row < top {
            self.scroll_by(pane, selection::edge_scroll_lines(top - mouse.row), true);
            self.extend_selection_to(pane, grid, mouse.column, mouse.row);
            self.set_autoscroll(Some(Self::autoscroll_at(AutoscrollDirection::Up, mouse)));
        } else if mouse.row > bottom {
            self.scroll_by(
                pane,
                selection::edge_scroll_lines(mouse.row - bottom),
                false,
            );
            self.extend_selection_to(pane, grid, mouse.column, mouse.row);
            self.set_autoscroll(Some(Self::autoscroll_at(AutoscrollDirection::Down, mouse)));
        } else if mouse.row == top {
            self.set_autoscroll(Some(Self::autoscroll_at(AutoscrollDirection::Up, mouse)));
        } else if mouse.row == bottom {
            self.set_autoscroll(Some(Self::autoscroll_at(AutoscrollDirection::Down, mouse)));
        } else {
            self.set_autoscroll(None);
        }
    }

    /// An autoscroll record for the pointer's current position.
    fn autoscroll_at(direction: AutoscrollDirection, mouse: MouseEvent) -> Autoscroll {
        Autoscroll::builder()
            .direction(direction)
            .column(mouse.column)
            .row(mouse.row)
            .build()
    }

    /// Completes the held gesture: a bare click clears, a finalized word copy
    /// keeps its feedback highlight, and a drag copies the spanned text and
    /// clears (herdr's copy-on-select).
    fn handle_left_release(&mut self, pane: PaneId) {
        self.set_autoscroll(None);
        let Some(active) = self.selection else {
            return;
        };
        if active.pane() != pane || active.is_click() {
            self.selection = None;
            return;
        }
        if active.is_dragging() {
            let text = self.extract_selection_text(pane, &active);
            if let Some(text) = text.filter(|text| !text.is_empty()) {
                self.pending_clipboard = Some(text);
            }
            self.selection = None;
        }
    }

    /// The wheel scrolls the pane: an in-progress selection keeps extending
    /// under it (herdr), an alternate-screen child receives cursor keys, and
    /// otherwise the scrollback offset moves.
    fn handle_wheel(&mut self, pane: PaneId, main_area: Rect, mouse: MouseEvent, up: bool) {
        if !main_area.contains(Position::new(mouse.column, mouse.row)) {
            return;
        }
        let selecting = self
            .selection
            .as_ref()
            .is_some_and(|active| active.pane() == pane && active.is_in_progress());
        if !selecting {
            let Some(target) = self.panes.get_mut(&pane) else {
                return;
            };
            let screen = target.parser.screen();
            if screen.alternate_screen() {
                let bytes = input::wheel_arrow(up, screen.application_cursor());
                if let Some(handle) = target.handle.as_mut() {
                    for _ in 0..WHEEL_SCROLL_LINES {
                        let _ = handle.write_input(bytes);
                    }
                }
                return;
            }
        }
        self.scroll_by(pane, WHEEL_SCROLL_LINES, up);
        if selecting && let Some(grid) = self.pane_grid(pane, main_area) {
            self.extend_selection_to(pane, grid, mouse.column, mouse.row);
        }
    }

    /// Moves the selection head to the pane cell nearest the pointer.
    fn extend_selection_to(&mut self, pane: PaneId, grid: Rect, column: u16, row: u16) {
        let Some(cell) = selection::nearest_cell(grid, column, row) else {
            return;
        };
        let Some(metrics) = self.pane_scroll_metrics(pane) else {
            return;
        };
        if let Some(active) = self.selection.as_mut() {
            active.extend_to(metrics.buffer_cell(cell));
        }
    }

    /// Moves the pane's scrollback offset by `lines`.
    fn scroll_by(&mut self, pane: PaneId, lines: usize, up: bool) {
        let Some(target) = self.panes.get_mut(&pane) else {
            return;
        };
        let screen = target.parser.screen_mut();
        let offset = if up {
            screen.scrollback().saturating_add(lines)
        } else {
            screen.scrollback().saturating_sub(lines)
        };
        screen.set_scrollback(offset);
    }

    /// Replaces the autoscroll state, arming its tick when one starts and
    /// disarming it when it ends.
    fn set_autoscroll(&mut self, autoscroll: Option<Autoscroll>) {
        match (&self.selection_autoscroll, &autoscroll) {
            (None, Some(_)) => {
                self.autoscroll_deadline = Some(Instant::now() + SELECTION_AUTOSCROLL_INTERVAL);
            },
            (_, None) => self.autoscroll_deadline = None,
            _ => {},
        }
        self.selection_autoscroll = autoscroll;
    }

    /// Drops every piece of selection state: highlight, autoscroll, feedback
    /// deadline, and the double-click candidate.
    fn clear_selection(&mut self) {
        self.selection = None;
        self.selection_view = None;
        self.set_autoscroll(None);
        self.selection_clear_deadline = None;
        self.last_pane_click = None;
    }

    /// The next moment the selection needs servicing: an autoscroll tick or a
    /// word-highlight expiry.
    pub fn next_selection_deadline(&self) -> Option<Instant> {
        match (self.autoscroll_deadline, self.selection_clear_deadline) {
            (Some(tick), Some(clear)) => Some(tick.min(clear)),
            (tick, clear) => tick.or(clear),
        }
    }

    /// Advances selection timers: expires the word-copy highlight and steps an
    /// active edge autoscroll. Returns whether a redraw is needed.
    pub fn advance_selection(&mut self, now: Instant) -> bool {
        let mut redraw = false;
        if self
            .selection_clear_deadline
            .is_some_and(|deadline| deadline <= now)
        {
            self.selection_clear_deadline = None;
            self.selection = None;
            redraw = true;
        }
        if self
            .autoscroll_deadline
            .is_some_and(|deadline| deadline <= now)
        {
            redraw |= self.tick_autoscroll(now);
        }
        redraw
    }

    /// One autoscroll step (herdr's 30 ms cadence): scroll a line toward the
    /// drag direction, stop at the buffer edge, and re-extend the selection
    /// at the pointer's last known spot.
    fn tick_autoscroll(&mut self, now: Instant) -> bool {
        let Some(autoscroll) = self.selection_autoscroll else {
            self.autoscroll_deadline = None;
            return false;
        };
        let Some(pane) = self
            .selection
            .as_ref()
            .filter(|active| active.is_dragging())
            .map(Selection::pane)
        else {
            self.set_autoscroll(None);
            return false;
        };
        let Some(metrics) = self.pane_scroll_metrics(pane) else {
            self.set_autoscroll(None);
            return false;
        };
        let at_edge = match autoscroll.direction() {
            AutoscrollDirection::Up => metrics.offset() >= metrics.len(),
            AutoscrollDirection::Down => metrics.offset() == 0,
        };
        if at_edge {
            self.set_autoscroll(None);
            return false;
        }
        self.scroll_by(pane, 1, autoscroll.direction() == AutoscrollDirection::Up);
        let (_, main_area, _) = areas(self.frame_area);
        if let Some(grid) = self.pane_grid(pane, main_area) {
            self.extend_selection_to(pane, grid, autoscroll.column(), autoscroll.row());
        }
        self.autoscroll_deadline = Some(now + SELECTION_AUTOSCROLL_INTERVAL);
        true
    }

    /// Reads the selected text, walking the scrollback in viewport-sized
    /// chunks when the span extends beyond the visible screen.
    fn extract_selection_text(&mut self, pane: PaneId, active: &Selection) -> Option<String> {
        let target = self.panes.get_mut(&pane)?;
        let screen = target.parser.screen_mut();
        let (rows, columns) = screen.size();
        if rows == 0 || columns == 0 {
            return None;
        }
        let saved = screen.scrollback();
        screen.set_scrollback(usize::MAX);
        let len = screen.scrollback();
        let (start, end) = active.span();
        let max_row = len + usize::from(rows) - 1;
        if start.row() > max_row {
            screen.set_scrollback(saved);
            return None;
        }
        let end_row = end.row().min(max_row);
        let mut text = String::new();
        let mut chunk_top = start.row();
        loop {
            screen.set_scrollback(len.saturating_sub(chunk_top));
            let viewport_top = len - screen.scrollback();
            let last_visible = viewport_top + usize::from(rows) - 1;
            let chunk_end = end_row.min(last_visible);
            let start_column = if chunk_top == start.row() {
                start.column()
            } else {
                0
            };
            let end_column = if chunk_end == end_row {
                end.column().saturating_add(1).min(columns)
            } else {
                columns
            };
            text.push_str(&screen.contents_between(
                (chunk_top - viewport_top) as u16,
                start_column,
                (chunk_end - viewport_top) as u16,
                end_column,
            ));
            if chunk_end >= end_row {
                break;
            }
            text.push('\n');
            chunk_top = last_visible + 1;
        }
        screen.set_scrollback(saved);
        Some(text)
    }

    /// Where the pane's viewport sits in its scrollback. Learning the total
    /// briefly clamps the offset to its maximum, so this needs `&mut`.
    fn pane_scroll_metrics(&mut self, pane: PaneId) -> Option<ScrollMetrics> {
        let target = self.panes.get_mut(&pane)?;
        let screen = target.parser.screen_mut();
        let offset = screen.scrollback();
        screen.set_scrollback(usize::MAX);
        let len = screen.scrollback();
        screen.set_scrollback(offset);
        Some(ScrollMetrics::builder().offset(offset).len(len).build())
    }

    /// Recomputes the viewport span of the active selection for the next
    /// frame, so rendering itself stays immutable.
    pub fn refresh_selection_view(&mut self) {
        self.selection_view = None;
        let Some(active) = self.selection else {
            return;
        };
        if Some(active.pane()) != self.selected_pane() {
            return;
        }
        let Some(metrics) = self.pane_scroll_metrics(active.pane()) else {
            return;
        };
        let Some(target) = self.panes.get(&active.pane()) else {
            return;
        };
        let (rows, columns) = target.parser.screen().size();
        self.selection_view = active.viewport_span(metrics.viewport_top(), rows, columns);
    }

    /// Tracks which pointer shape the hovered region wants and queues the
    /// OSC 22 update when it changes: an I-beam over selectable pane text,
    /// the regular arrow everywhere else.
    fn refresh_pointer_shape(&mut self, main_area: Rect, mouse: MouseEvent) {
        let over_text = self.selected_pane().is_some_and(|pane| {
            !self.pane_wants_mouse(pane)
                && self
                    .pane_grid(pane, main_area)
                    .is_some_and(|grid| grid.contains(Position::new(mouse.column, mouse.row)))
        });
        let shape = if over_text {
            PointerShape::Text
        } else {
            PointerShape::Default
        };
        if shape != self.pointer_shape {
            self.pointer_shape = shape;
            self.pending_pointer_shape = Some(shape);
        }
    }

    /// Whether the pane's live child explicitly enabled xterm mouse reporting.
    fn pane_wants_mouse(&self, pane: PaneId) -> bool {
        self.panes.get(&pane).is_some_and(|target| {
            target.handle.is_some()
                && target.parser.screen().mouse_protocol_mode() != MouseProtocolMode::None
        })
    }

    /// Forwards pointer input to the terminal that requested xterm mouse mode.
    fn forward_mouse_to_pane(&mut self, pane: PaneId, area: Rect, mouse: MouseEvent) {
        let Some(target) = self.panes.get(&pane) else {
            return;
        };
        let screen = target.parser.screen();
        let mode = screen.mouse_protocol_mode();
        let Some((column, row)) =
            Self::relative_mouse_position(area, mouse.column, mouse.row, screen)
        else {
            return;
        };
        let bytes = input::encode_mouse(mouse, column, row, mode, screen.mouse_protocol_encoding());
        if let Some(bytes) = bytes
            && let Some(target) = self.panes.get_mut(&pane)
            && let Some(handle) = target.handle.as_mut()
        {
            let _ = handle.write_input(&bytes);
        }
    }

    /// Maps an outer-terminal mouse coordinate into the child terminal grid.
    fn relative_mouse_position(
        area: Rect,
        column: u16,
        row: u16,
        screen: &Screen,
    ) -> Option<(u16, u16)> {
        let x = column.checked_sub(area.x + BORDER_THICKNESS)?;
        let y = row.checked_sub(area.y + BORDER_THICKNESS)?;
        let (rows, columns) = screen.size();
        (x < columns && y < rows).then_some((x, y))
    }

    /// The absolute rectangle of the pane's visible cells: the main area inside
    /// its border, intersected with the child screen size.
    fn pane_grid(&self, pane: PaneId, main_area: Rect) -> Option<Rect> {
        let target = self.panes.get(&pane)?;
        let (rows, columns) = target.parser.screen().size();
        let width = main_area
            .width
            .saturating_sub(BORDER_THICKNESS * 2)
            .min(columns);
        let height = main_area
            .height
            .saturating_sub(BORDER_THICKNESS * 2)
            .min(rows);
        (width > 0 && height > 0).then(|| {
            Rect::new(
                main_area.x + BORDER_THICKNESS,
                main_area.y + BORDER_THICKNESS,
                width,
                height,
            )
        })
    }

    /// The sidebar display inputs shared by rendering and click hit-testing.
    fn sidebar_context(&self) -> (String, Vec<String>, sidebar::SidebarSelection) {
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
        (active_label, other_projects, selection)
    }

    /// Draws the whole UI: sidebar, focused terminal, and status bar.
    pub fn render(&self, frame: &mut Frame) {
        let (sidebar_area, main_area, status_area) = areas(frame.area());
        let sidebar_focused = self.focus == Focus::Sidebar;
        let (active_label, other_projects, selection) = self.sidebar_context();
        let sidebar_state = sidebar::SidebarState::builder()
            .workspace(&self.workspace)
            .activity_frame(self.activity_frame)
            .focused(sidebar_focused)
            .active_project(&active_label)
            .other_projects(&other_projects)
            .selection(selection)
            .build();
        sidebar::render(frame, sidebar_area, &sidebar_state);
        let (title, screen) = self.focused_view();
        terminal_pane::render(
            frame,
            main_area,
            &title,
            screen,
            !sidebar_focused,
            self.selection_view,
            self.selection_style,
        );
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
            self.notice
                .as_deref()
                .map(|notice| (notice, NoticeTone::Error)),
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
            return if self.selected_is_agent_session() {
                StatusContext::TerminalAgentSession
            } else {
                StatusContext::Terminal
            };
        }
        match self.project_cursor {
            Some(_) => StatusContext::Project,
            None if self.workspace.is_empty() => StatusContext::Empty,
            None if self.selected_is_agent_session() => StatusContext::AgentSession,
            None => StatusContext::Process,
        }
    }

    /// Whether the selected row is a runtime-managed agent session.
    fn selected_is_agent_session(&self) -> bool {
        self.workspace.selected_process().is_some_and(|process| {
            *process.kind() == ProcessKind::Agent && *process.origin() == ProcessOrigin::Session
        })
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

    /// Handles a key: leader chord, command, or forward to the focused pane.
    fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        // A key press dismisses any transient notice from the previous action
        // and, like herdr, any lingering selection state.
        self.notice = None;
        self.clear_selection();
        match &self.overlay {
            // Help is a read-only reference; any key closes it.
            Some(Overlay::Help) => {
                self.overlay = None;
                return;
            },
            Some(
                Overlay::ConfirmOverwrite { .. }
                | Overlay::ConfirmRemoval { .. }
                | Overlay::ConfirmSessionClose { .. },
            ) => {
                self.handle_confirm_key(key);
                return;
            },
            Some(Overlay::Form(_)) => {
                self.handle_form_key(key);
                return;
            },
            Some(Overlay::AgentPicker(_)) => {
                self.handle_agent_picker_key(key);
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
            KeyCode::Char('d') => self.confirm_close_selected_session(),
            KeyCode::Char('u') => self.reopen_last_closed_session(),
            KeyCode::Char('t') if self.project_cursor.is_none() => self.toggle_selected_autostart(),
            KeyCode::Char('s') if self.project_cursor.is_none() => self.toggle_selected(),
            KeyCode::Char('r') if self.project_cursor.is_none() => self.restart_selected(),
            KeyCode::Char('p') if self.project_cursor.is_none() => self.toggle_pause_selected(),
            KeyCode::Char('x') if self.project_cursor.is_none() => self.force_stop_selected(),
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
        let config_path = match path::registered_config_path(&project) {
            Ok(config_path) => config_path,
            Err(error) => {
                self.notice = Some(error.to_string());
                return;
            },
        };
        match self.registry.workspace(&config_path) {
            Ok(config) => {
                self.project_cursor = None;
                self.begin_switch(config, config_path);
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

    /// Confirms closing the selected runtime agent session. A configured
    /// agent remains pinned because its lifecycle is owned by `muster.yml`.
    fn confirm_close_selected_session(&mut self) {
        let Some(process) = self.workspace.selected_process() else {
            return;
        };
        if *process.kind() != ProcessKind::Agent {
            return;
        }
        if *process.origin() != ProcessOrigin::Session {
            self.notice = Some(CONFIGURED_AGENT_CLOSE_UNAVAILABLE.to_string());
            return;
        }
        let pane = *process.id();
        let message = format!("Close agent session {}?", process.name().as_ref());
        if self
            .panes
            .get(&pane)
            .is_some_and(|target| target.handle.is_some())
        {
            self.overlay = Some(Overlay::ConfirmSessionClose { message, pane });
        } else {
            self.close_agent_session(pane);
        }
    }

    /// Force-kills one runtime agent session and records it closed only after
    /// stop delivery succeeds. Its row retires on exit, or immediately when it
    /// was already stopped, with focus returned to the sidebar first.
    fn close_agent_session(&mut self, pane: PaneId) {
        self.focus = Focus::Sidebar;
        let session_id = self
            .workspace
            .process(pane)
            .and_then(|process| process.agent_session_id().clone());
        let alive = self
            .panes
            .get(&pane)
            .is_some_and(|target| target.handle.is_some());
        if !alive {
            if self.persist_agent_session_state(session_id.as_ref(), AgentSessionState::Closed) {
                self.pending_session_reopens.remove(&pane);
                self.retire_pane(pane);
            }
            return;
        }
        let delivered = self
            .panes
            .get_mut(&pane)
            .and_then(|target| target.handle.as_mut().map(|handle| handle.kill().is_ok()))
            .unwrap_or(false);
        if !delivered {
            self.notice = Some(STOP_DELIVERY_FAILED_NOTICE.to_string());
            return;
        }
        self.workspace.set_state(pane, ProcessState::Stopping);
        if !self.persist_agent_session_state(session_id.as_ref(), AgentSessionState::Closed) {
            return;
        }
        self.pending_session_reopens.remove(&pane);
        if let Some(target) = self.panes.get_mut(&pane) {
            target.config_membership = ConfigMembership::RetireOnExit;
        }
    }

    /// Persists one session lifecycle transition and exposes adapter failures
    /// without allowing the in-memory transition to continue.
    fn persist_agent_session_state(
        &mut self,
        session_id: Option<&AgentSessionId>,
        state: AgentSessionState,
    ) -> bool {
        let (Some(store), Some(session_id)) = (&self.agent_session_store, session_id) else {
            return true;
        };
        if let Err(error) = store.set_state(session_id, state) {
            self.notice = Some(format!("{AGENT_SESSION_STORE_ERROR}: {error}"));
            self.overlay = None;
            return false;
        }
        true
    }

    /// Whether `project` is the unsaved launched-project row synthesized for the
    /// tree rather than a registered project.
    fn is_synthetic_launched(&self, project: &Project) -> bool {
        self.launched_project_membership == LaunchedProjectMembership::Synthetic
            && Self::same_config_location(project.config(), &self.launched_config)
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
        if *process.origin() == ProcessOrigin::Session {
            self.notice = Some(SESSION_AUTOSTART_UNAVAILABLE.to_string());
            return;
        }
        let pane = *process.id();
        let autostart = !*process.autostart();
        let target = ProcessSpecMatcher::of(process);
        let occurrence = self
            .workspace
            .processes()
            .iter()
            .filter(|candidate| *candidate.origin() == ProcessOrigin::Configured)
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
        projects.retain(|project| !Self::same_config_location(project.config(), config_path));
        match self.registry.save(&projects) {
            Ok(()) => {
                self.project_cursor = None;
                self.refresh_projects();
            },
            Err(error) => self.notice = Some(error.to_string()),
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
            KeyCode::Char('x') => self.force_stop_selected(),
            KeyCode::Char('d') => self.confirm_close_selected_session(),
            KeyCode::Char('u') => self.reopen_last_closed_session(),
            KeyCode::Char('?') => self.overlay = Some(Overlay::Help),
            _ => {},
        }
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
            Some(Overlay::Form(form)) => form.switcher,
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

    /// Opens the first add-process step, where the process kind is selected.
    fn open_add_process_form(&mut self) {
        let form = Form::new(ADD_PROCESS_TITLE, vec![Field::choice(
            KIND_FIELD,
            &KIND_OPTIONS,
        )]);
        self.open_form(form, FormIntent::ChooseProcessKind);
    }

    /// Opens the quick agent picker with resumable history and fresh presets.
    fn open_agent_picker(&mut self) {
        let project = self.current_config.clone();
        let (mut items, error) = match self.agent_sessions() {
            Ok(sessions) => (
                sessions
                    .into_iter()
                    .rev()
                    .filter(|session| {
                        *session.state() == AgentSessionState::Closed
                            && session.restore_command().is_some()
                            && project.as_ref().is_some_and(|project| {
                                Self::same_config_location(session.project(), project)
                            })
                    })
                    .take(RECENT_AGENT_LIMIT)
                    .map(Box::new)
                    .map(AgentPickerItem::Recent)
                    .collect::<Vec<_>>(),
                None,
            ),
            Err(error) => (Vec::new(), Some(error.to_string())),
        };
        items.extend(AgentTool::options().map(AgentPickerItem::New));
        self.overlay = Some(Overlay::AgentPicker(AgentPicker {
            items,
            selected: 0,
            error,
        }));
    }

    /// Handles navigation, quick launch, customization, and cancel in the agent
    /// picker.
    fn handle_agent_picker_key(&mut self, key: KeyEvent) {
        let Some(Overlay::AgentPicker(picker)) = &self.overlay else {
            return;
        };
        let count = picker.items.len();
        let selected = picker.selected;
        match key.code {
            KeyCode::Esc => self.overlay = None,
            KeyCode::Char('j') | KeyCode::Down if count > 0 => {
                if let Some(Overlay::AgentPicker(picker)) = &mut self.overlay {
                    picker.selected = (selected + 1) % count;
                }
            },
            KeyCode::Char('k') | KeyCode::Up if count > 0 => {
                if let Some(Overlay::AgentPicker(picker)) = &mut self.overlay {
                    picker.selected = selected.checked_sub(1).unwrap_or(count - 1);
                }
            },
            KeyCode::Enter => {
                let item = match &self.overlay {
                    Some(Overlay::AgentPicker(picker)) => picker.items.get(selected).cloned(),
                    _ => None,
                };
                match item {
                    Some(AgentPickerItem::Recent(session)) => {
                        self.overlay = None;
                        self.reopen_agent_session(session.id());
                    },
                    Some(AgentPickerItem::New(AgentTool::Custom)) => {
                        self.open_agent_session_form(AgentTool::Custom);
                    },
                    Some(AgentPickerItem::New(tool)) => {
                        self.overlay = None;
                        self.create_agent_session(tool, None, None, None);
                    },
                    None => {},
                }
            },
            KeyCode::Char('e') => {
                let tool = match &self.overlay {
                    Some(Overlay::AgentPicker(picker)) => picker.items.get(selected),
                    _ => None,
                };
                if let Some(AgentPickerItem::New(tool)) = tool {
                    self.open_agent_session_form(*tool);
                }
            },
            _ => {},
        }
    }

    /// Opens advanced customization for a fresh agent provider.
    fn open_agent_session_form(&mut self, tool: AgentTool) {
        let tool_options = AgentTool::options()
            .map(|tool| tool.to_string())
            .collect::<Vec<_>>();
        let tool_option_refs = tool_options.iter().map(String::as_str).collect::<Vec<_>>();
        let form = Form::new(ADD_AGENT_TITLE, vec![
            Field::choice_value(TOOL_FIELD, &tool_option_refs, tool.as_ref()),
            Field::text(SESSION_NAME_FIELD),
            Field::text(AGENT_COMMAND_FIELD),
            Field::text(AGENT_RESUME_FIELD),
        ]);
        self.open_form(form, FormIntent::LaunchAgentSession);
    }

    /// Opens the persistent terminal or command form.
    fn open_configured_process_form(&mut self, kind: ProcessKind) {
        let form = Form::new(ADD_PROCESS_TITLE, vec![
            Field::text(NAME_FIELD),
            Field::text(COMMAND_FIELD),
        ]);
        self.open_form(form, FormIntent::AddConfiguredProcess(kind));
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
        match self.registry.save(&projects) {
            Ok(()) => {
                self.refresh_projects();
                self.refresh_switcher();
            },
            Err(error) => {
                if let Some(switcher) = self.switcher_mut() {
                    switcher.error = Some(error.to_string());
                }
            },
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
            FormIntent::ChooseProcessKind => self.choose_process_kind(&values),
            FormIntent::LaunchAgentSession => self.launch_agent_session(&values),
            FormIntent::AddConfiguredProcess(kind) => {
                self.add_configured_process(kind, &values);
            },
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
                    Some(Overlay::ConfirmSessionClose { pane, .. }) => {
                        self.close_agent_session(pane);
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

    /// Reports an error in the active modal, falling back to a status notice.
    fn report_error(&mut self, message: &str) {
        if let Some(modal) = self.form_mut() {
            modal.error = Some(message.to_string());
        } else if let Some(switcher) = self.switcher_mut() {
            switcher.error = Some(message.to_string());
        } else if let Some(Overlay::AgentPicker(picker)) = &mut self.overlay {
            picker.error = Some(message.to_string());
        } else {
            self.notice = Some(message.to_string());
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
        let project = Project::builder()
            .name(project.name().clone())
            .config(path::absolutize(project.config()))
            .build();
        projects
            .retain(|existing| !Self::same_config_location(existing.config(), project.config()));
        projects.push(project);
        match self.registry.save(&projects) {
            Ok(()) => true,
            Err(error) => {
                self.report_error(&error.to_string());
                false
            },
        }
    }

    /// Advances the add flow to the selected kind's specific form.
    fn choose_process_kind(&mut self, values: &[String]) {
        match values.first().map(String::as_str) {
            Some(KIND_AGENT) => self.open_agent_picker(),
            Some(KIND_TERMINAL) => self.open_configured_process_form(ProcessKind::Terminal),
            Some(KIND_COMMAND) => self.open_configured_process_form(ProcessKind::Command),
            _ => {},
        }
    }

    /// Launches a durable coding-agent session without modifying `muster.yml`.
    fn launch_agent_session(&mut self, values: &[String]) {
        let (Some(tool), Some(name), Some(command)) =
            (values.first(), values.get(1), values.get(2))
        else {
            return;
        };
        let Ok(tool) = tool.parse::<AgentTool>() else {
            return;
        };
        let name = if name.trim().is_empty() {
            None
        } else {
            ProcessName::try_new(name).ok()
        };
        let command = if command.trim().is_empty() {
            None
        } else {
            CommandLine::try_new(command).ok()
        };
        let resume = values.get(3).and_then(|resume| {
            (!resume.trim().is_empty())
                .then(|| CommandLine::try_new(resume))
                .and_then(Result::ok)
        });
        self.create_agent_session(tool, name, command, resume);
    }

    /// Creates, persists, launches, and attaches a new agent conversation.
    fn create_agent_session(
        &mut self,
        tool: AgentTool,
        name: Option<ProcessName>,
        command: Option<CommandLine>,
        resume_command: Option<CommandLine>,
    ) {
        if resume_command
            .as_ref()
            .is_some_and(|template| !AgentSession::resume_template_is_valid(template))
        {
            self.report_error(AGENT_RESUME_TEMPLATE_INVALID);
            return;
        }
        let Some(project) = self.current_config.clone() else {
            return;
        };
        let command = match command.or_else(|| {
            tool.default_command()
                .and_then(|command| CommandLine::try_new(command).ok())
        }) {
            Some(command) => command,
            None => {
                self.report_error(AGENT_COMMAND_REQUIRED);
                return;
            },
        };
        if resume_command.is_none() {
            let Ok(validation_id) = NativeSessionId::try_new(RESUME_VALIDATION_ID) else {
                self.report_error(AGENT_SESSION_STORE_ERROR);
                return;
            };
            if tool != AgentTool::Custom && tool.resume_command(&command, &validation_id).is_none()
            {
                self.report_error(AGENT_COMPOUND_RESUME_REQUIRED);
                return;
            }
        }
        let Ok(id) = AgentSessionId::generate() else {
            self.report_error(AGENT_SESSION_STORE_ERROR);
            return;
        };
        let name = match name.or_else(|| self.generated_agent_name(tool, &id)) {
            Some(name) => name,
            None => {
                self.report_error(AGENT_SESSION_STORE_ERROR);
                return;
            },
        };
        let Some(launch_command) = tool.new_session_command(&command, &id) else {
            self.report_error(AGENT_COMMAND_REQUIRED);
            return;
        };
        let session = AgentSession::builder()
            .id(id.clone())
            .name(name)
            .tool(tool)
            .project(project)
            .launch_command(command.clone())
            .resume_command(resume_command)
            .state(AgentSessionState::Pending)
            .build();
        if let Some(store) = &self.agent_session_store
            && let Err(error) = store.upsert(&session)
        {
            self.report_error(&format!("{AGENT_SESSION_STORE_ERROR}: {error}"));
            return;
        }
        self.insert_agent_session(
            &session,
            AgentSessionActivation::StartAttached(launch_command),
        );
    }

    /// Generates a friendly non-identifying name, avoiding active-row
    /// collisions when possible and falling back to the provider plus UUID.
    fn generated_agent_name(&self, tool: AgentTool, id: &AgentSessionId) -> Option<ProcessName> {
        for _ in 0..GENERATED_NAME_ATTEMPTS {
            let generated: String = FirstName().fake();
            if let Ok(name) = ProcessName::try_new(generated)
                && !self
                    .workspace
                    .processes()
                    .iter()
                    .any(|process| process.name() == &name)
            {
                return Some(name);
            }
        }
        let suffix: String = id.as_ref().chars().take(8).collect();
        ProcessName::try_new(format!("{tool} {suffix}")).ok()
    }

    /// Inserts one persisted session as a process, applying its requested
    /// stopped, detached-start, or attached-start activation.
    fn insert_agent_session(&mut self, session: &AgentSession, activation: AgentSessionActivation) {
        if let Some(process) = self
            .workspace
            .processes()
            .iter()
            .find(|process| process.agent_session_id().as_ref() == Some(session.id()))
        {
            let pane = *process.id();
            if let Some(index) = self.workspace.position_of(pane) {
                self.workspace.select_at(index);
            }
            if let Some(command) = activation.command().cloned()
                && self
                    .panes
                    .get(&pane)
                    .is_none_or(|pane| pane.handle.is_none())
            {
                self.spawn(pane, Some(command), session.working_dir().clone());
            }
            if activation.should_attach() {
                self.focus = Focus::Terminal;
            }
            return;
        }
        let pane = self.next_pane_id();
        let process = Process::builder()
            .id(pane)
            .name(session.name().clone())
            .kind(ProcessKind::Agent)
            .agent_tool(Some(*session.tool()))
            .agent_session_id(Some(session.id().clone()))
            .origin(ProcessOrigin::Session)
            .command(activation.command().cloned())
            .working_dir(session.working_dir().clone())
            .autostart(activation.command().is_some())
            .build();
        let selected = self.workspace.insert_in_section(process);
        self.workspace.select_at(selected);
        self.project_cursor = None;
        self.overlay = None;
        if let Some(command) = activation.command().cloned() {
            self.spawn(pane, Some(command), session.working_dir().clone());
        }
        if activation.should_attach() {
            self.focus = Focus::Terminal;
        }
    }

    /// Reopens a persisted session by ID with its provider-native command.
    fn reopen_agent_session(&mut self, id: &AgentSessionId) {
        if self.pending_switch.is_some() {
            self.notice = Some(PROJECT_SWITCH_IN_PROGRESS.to_string());
            return;
        }
        let session = match self.agent_sessions() {
            Ok(sessions) => sessions.into_iter().find(|session| session.id() == id),
            Err(error) => {
                self.notice = Some(format!("{AGENT_SESSION_STORE_ERROR}: {error}"));
                return;
            },
        };
        let Some(session) = session else {
            self.notice = Some(NO_RECENT_AGENT_SESSION.to_string());
            return;
        };
        let Some(command) = session.restore_command() else {
            self.notice = Some(AGENT_SESSION_NOT_RESUMABLE.to_string());
            return;
        };
        if let Some(store) = &self.agent_session_store
            && let Err(error) = store.set_state(session.id(), AgentSessionState::Open)
        {
            self.notice = Some(format!("{AGENT_SESSION_STORE_ERROR}: {error}"));
            return;
        }
        let closing = self.workspace.processes().iter().find_map(|process| {
            (process.agent_session_id().as_ref() == Some(session.id()))
                .then_some(*process.id())
                .filter(|pane| {
                    self.panes.get(pane).is_some_and(|target| {
                        target.handle.is_some()
                            && target.config_membership == ConfigMembership::RetireOnExit
                    })
                })
        });
        if let Some(pane) = closing {
            self.pending_session_reopens
                .insert(pane, session.id().clone());
            return;
        }
        self.insert_agent_session(&session, AgentSessionActivation::StartAttached(command));
    }

    /// Reopens the newest closed resumable session owned by this workspace.
    fn reopen_last_closed_session(&mut self) {
        let Some(project) = self.current_config.as_ref() else {
            return;
        };
        let sessions = match self.agent_sessions() {
            Ok(sessions) => sessions,
            Err(error) => {
                self.notice = Some(format!("{AGENT_SESSION_STORE_ERROR}: {error}"));
                return;
            },
        };
        let belongs_to_project = |session: &AgentSession| {
            *session.state() == AgentSessionState::Closed
                && Self::same_config_location(session.project(), project)
        };
        let has_closed = sessions.iter().any(&belongs_to_project);
        let session = sessions
            .iter()
            .rev()
            .filter(|session| belongs_to_project(session))
            .find(|session| session.restore_command().is_some());
        let Some(session) = session else {
            self.notice = Some(
                if has_closed {
                    AGENT_SESSION_NOT_RESUMABLE
                } else {
                    NO_RECENT_AGENT_SESSION
                }
                .to_string(),
            );
            return;
        };
        let id = session.id().clone();
        self.reopen_agent_session(&id);
    }

    /// Loads durable session history, returning an empty list in test or custom
    /// compositions that deliberately omit the store.
    fn agent_sessions(&self) -> Result<Vec<AgentSession>, ConfigError> {
        self.agent_session_store
            .as_ref()
            .map_or_else(|| Ok(Vec::new()), |store| store.sessions())
    }

    /// Returns a pane id unused by configured and runtime processes.
    fn next_pane_id(&self) -> PaneId {
        let next = self
            .workspace
            .processes()
            .iter()
            .map(|process| process.id().into_inner())
            .max()
            .map_or(0, |pane| pane + 1);
        PaneId::new(next)
    }

    /// Adds a persistent terminal or command and reconciles it in place without
    /// interrupting existing configured processes or agent sessions.
    fn add_configured_process(&mut self, kind: ProcessKind, values: &[String]) {
        if kind == ProcessKind::Agent {
            return;
        }
        let (Some(name), Some(command)) = (values.first(), values.get(1)) else {
            return;
        };
        let Some(config_path) = self.current_config.clone() else {
            return;
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
        let target = ProcessSpecMatcher::of_spec(kind, &spec);
        // Route through the registry's locked read-modify-write, the same one
        // `muster run` uses, so an overlapping CLI add and this add cannot
        // silently discard each other.
        let mut updated = None;
        let mut target_occurrence = None;
        let update_result = {
            let mut append = |config: WorkspaceConfig| {
                let config = match kind {
                    ProcessKind::Agent => config,
                    ProcessKind::Terminal => {
                        let mut specs = config.terminals().clone();
                        target_occurrence =
                            Some(specs.iter().filter(|spec| target.matches(spec)).count());
                        specs.push(spec.clone());
                        config.with_terminals(specs)
                    },
                    ProcessKind::Command => {
                        let mut specs = config.commands().clone();
                        target_occurrence =
                            Some(specs.iter().filter(|spec| target.matches(spec)).count());
                        specs.push(spec.clone());
                        config.with_commands(specs)
                    },
                };
                updated = Some(config.clone());
                config
            };
            self.registry.update_workspace(&config_path, &mut append)
        };
        if update_result.is_err() {
            self.report_error(WORKSPACE_SAVE_ERROR);
            return;
        }
        let Some(config) = updated else {
            self.report_error(WORKSPACE_SAVE_ERROR);
            return;
        };
        self.overlay = None;
        self.reconcile_config(&config);
        let launch = target_occurrence
            .and_then(|occurrence| self.configured_process_for_spec_occurrence(&target, occurrence))
            .filter(|process| *process.autostart() && !process.state().is_active())
            .map(|process| {
                (
                    *process.id(),
                    process.command().clone(),
                    process.working_dir().clone(),
                )
            });
        if let Some((pane, command, cwd)) = launch {
            self.spawn(pane, command, cwd);
        }
    }

    /// Returns the tracked configured process representing one occurrence of a
    /// spec identity after reconciliation.
    fn configured_process_for_spec_occurrence(
        &self,
        target: &ProcessSpecMatcher,
        occurrence: usize,
    ) -> Option<&Process> {
        self.workspace
            .processes()
            .iter()
            .filter(|process| *process.origin() == ProcessOrigin::Configured)
            .filter(|process| target.matches_process(process))
            .filter(|process| {
                self.panes
                    .get(process.id())
                    .is_none_or(|pane| pane.config_membership == ConfigMembership::Tracked)
            })
            .nth(occurrence)
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

    /// Forwards an encoded key to the focused pane's PTY, if it is alive.
    /// Typing snaps a scrolled-back view down to the live screen first.
    fn forward_key(&mut self, key: KeyEvent) {
        let Some(bytes) = input::encode_key(key) else {
            return;
        };
        if let Some(pane) = self.selected_pane()
            && let Some(target) = self.panes.get_mut(&pane)
        {
            target.parser.screen_mut().set_scrollback(0);
            if let Some(handle) = target.handle.as_mut() {
                let _ = handle.write_input(&bytes);
            }
        }
    }

    /// Resizes every live pane's PTY and parser to match `area`.
    fn resize(&mut self, area: Rect) {
        self.frame_area = area;
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

/// A display name for a project taken from its config path: the parent
/// directory's name, else the app name. The path is absolutized first so a
/// relative default like `muster.yml` resolves to the current directory's name
/// rather than losing its parent, while a symlink retains its workspace alias.
fn label_from_config(config: &Path) -> String {
    path::absolutize(config)
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
    use crossterm::event::{MouseButton, MouseEventKind};

    use super::*;
    use crate::{
        adapter::{process_identity::LocalProcessIdentity, tui::activity::OUTPUT_IDLE_TIMEOUT},
        domain::{
            agent_session::{AgentProcessId, AgentProcessStartToken, NativeSessionId},
            config::{ConfigError, ProcessSpec},
            port::OutputSink,
            process::{Process, ProcessKind, RestartPolicy, StopSignal},
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

    /// Shared in-memory durable state for agent-session lifecycle tests.
    #[derive(Clone, Default)]
    struct SessionRecorder {
        sessions: Rc<RefCell<Vec<AgentSession>>>,
    }

    /// An agent-session store with the same history ordering semantics as the
    /// YAML adapter, without touching platform state during TUI tests.
    struct FakeAgentSessionStore {
        recorder: SessionRecorder,
    }

    impl AgentSessionStore for FakeAgentSessionStore {
        fn sessions(&self) -> Result<Vec<AgentSession>, ConfigError> {
            Ok(self.recorder.sessions.borrow().clone())
        }

        fn state_file_path(&self) -> Result<Option<PathBuf>, ConfigError> {
            Ok(None)
        }

        fn upsert(&self, session: &AgentSession) -> Result<(), ConfigError> {
            let mut sessions = self.recorder.sessions.borrow_mut();
            sessions.retain(|candidate| candidate.id() != session.id());
            sessions.push(session.clone());
            Ok(())
        }

        fn set_state(
            &self,
            id: &AgentSessionId,
            state: AgentSessionState,
        ) -> Result<(), ConfigError> {
            let mut sessions = self.recorder.sessions.borrow_mut();
            let index = sessions
                .iter()
                .position(|session| session.id() == id)
                .ok_or_else(|| ConfigError::AgentSessionNotFound(id.clone()))?;
            let session = sessions.remove(index).with_state(state);
            sessions.push(session);
            Ok(())
        }

        fn set_owner_process_id(
            &self,
            id: &AgentSessionId,
            process_id: AgentProcessId,
            process_start_token: Option<AgentProcessStartToken>,
            wrapper_process_id: Option<AgentProcessId>,
        ) -> Result<(), ConfigError> {
            let mut sessions = self.recorder.sessions.borrow_mut();
            let session = sessions
                .iter_mut()
                .find(|session| session.id() == id)
                .ok_or_else(|| ConfigError::AgentSessionNotFound(id.clone()))?;
            *session = session.clone().with_launch_processes(
                process_id,
                process_start_token,
                wrapper_process_id,
            );
            Ok(())
        }

        fn capture_native_id(
            &self,
            id: &AgentSessionId,
            provider: AgentTool,
            process_id: AgentProcessId,
            parent_process_id: Option<AgentProcessId>,
            native_id: NativeSessionId,
        ) -> Result<(), ConfigError> {
            let mut sessions = self.recorder.sessions.borrow_mut();
            let session = sessions
                .iter_mut()
                .find(|session| session.id() == id)
                .ok_or_else(|| ConfigError::AgentSessionNotFound(id.clone()))?;
            if *session.tool() != provider {
                return Err(ConfigError::AgentSessionProviderMismatch {
                    id: id.clone(),
                    expected: *session.tool(),
                    reported: provider,
                });
            }
            let owns_process = session.owner_process_id().as_ref() == Some(&process_id);
            let is_wrapper_handoff = session.wrapper_process_id().is_some()
                && session.wrapper_process_id().as_ref() == parent_process_id.as_ref();
            if !owns_process && !is_wrapper_handoff {
                return Err(ConfigError::AgentSessionProcessMismatch {
                    id: id.clone(),
                    expected: *session.owner_process_id(),
                    reported: process_id,
                });
            }
            *session = session.clone().with_native_id(native_id);
            Ok(())
        }
    }

    /// A store that models an unavailable platform state directory.
    struct FailingAgentSessionStore;

    impl AgentSessionStore for FailingAgentSessionStore {
        fn sessions(&self) -> Result<Vec<AgentSession>, ConfigError> {
            Ok(Vec::new())
        }

        fn state_file_path(&self) -> Result<Option<PathBuf>, ConfigError> {
            Ok(None)
        }

        fn upsert(&self, _session: &AgentSession) -> Result<(), ConfigError> {
            Err(ConfigError::NoConfigDir)
        }

        fn set_state(
            &self,
            _id: &AgentSessionId,
            _state: AgentSessionState,
        ) -> Result<(), ConfigError> {
            Err(ConfigError::NoConfigDir)
        }

        fn set_owner_process_id(
            &self,
            _id: &AgentSessionId,
            _process_id: AgentProcessId,
            _process_start_token: Option<AgentProcessStartToken>,
            _wrapper_process_id: Option<AgentProcessId>,
        ) -> Result<(), ConfigError> {
            Err(ConfigError::NoConfigDir)
        }

        fn capture_native_id(
            &self,
            _id: &AgentSessionId,
            _provider: AgentTool,
            _process_id: AgentProcessId,
            _parent_process_id: Option<AgentProcessId>,
            _native_id: NativeSessionId,
        ) -> Result<(), ConfigError> {
            Err(ConfigError::NoConfigDir)
        }
    }

    /// A store that fails reads while preserving the rest of the port shape.
    struct UnreadableAgentSessionStore;

    impl AgentSessionStore for UnreadableAgentSessionStore {
        fn sessions(&self) -> Result<Vec<AgentSession>, ConfigError> {
            Err(ConfigError::NoConfigDir)
        }

        fn state_file_path(&self) -> Result<Option<PathBuf>, ConfigError> {
            Err(ConfigError::NoConfigDir)
        }

        fn upsert(&self, _session: &AgentSession) -> Result<(), ConfigError> {
            Err(ConfigError::NoConfigDir)
        }

        fn set_state(
            &self,
            _id: &AgentSessionId,
            _state: AgentSessionState,
        ) -> Result<(), ConfigError> {
            Err(ConfigError::NoConfigDir)
        }

        fn set_owner_process_id(
            &self,
            _id: &AgentSessionId,
            _process_id: AgentProcessId,
            _process_start_token: Option<AgentProcessStartToken>,
            _wrapper_process_id: Option<AgentProcessId>,
        ) -> Result<(), ConfigError> {
            Err(ConfigError::NoConfigDir)
        }

        fn capture_native_id(
            &self,
            _id: &AgentSessionId,
            _provider: AgentTool,
            _process_id: AgentProcessId,
            _parent_process_id: Option<AgentProcessId>,
            _native_id: NativeSessionId,
        ) -> Result<(), ConfigError> {
            Err(ConfigError::NoConfigDir)
        }
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

    /// Shared record of complete PTY requests for command and environment
    /// assertions.
    #[derive(Clone, Default)]
    struct SpawnRecorder {
        requests: Rc<RefCell<Vec<SpawnRequest>>>,
    }

    /// A successful runner that retains each spawn request.
    struct RequestRecordingRunner {
        recorder: SpawnRecorder,
    }

    impl ProcessRunner for RequestRecordingRunner {
        fn spawn(
            &self,
            request: SpawnRequest,
            _sink: Box<dyn OutputSink>,
        ) -> Result<Box<dyn ProcessHandle>, PtyError> {
            self.recorder.requests.borrow_mut().push(request);
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
        termination: Mutex<Option<(StopSignal, Duration)>>,
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

        fn terminate(&mut self, signal: StopSignal, grace: Duration) -> Result<(), PtyError> {
            self.signals.terminates.fetch_add(1, Ordering::SeqCst);
            *self.signals.termination.lock().unwrap() = Some((signal, grace));
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

        fn terminate(&mut self, _signal: StopSignal, _grace: Duration) -> Result<(), PtyError> {
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
    /// Grace period used to verify policy propagation without slowing tests.
    const TEST_STOP_GRACE: Duration = Duration::from_secs(7);

    /// Verifies absent and relative process directories are anchored to the
    /// workspace while an absolute directory remains unchanged.
    #[test]
    fn spawn_paths_are_anchored_to_the_workspace_config() {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let config = workspace.join(PROJECT_CONFIG_FILE);

        let (project, inherited) = App::resolve_spawn_paths(Some(&config), None);
        assert_eq!(project, Some(config.clone()));
        assert_eq!(inherited, Some(workspace.clone()));

        let relative = PathBuf::from("services").join("api");
        let (_, resolved) = App::resolve_spawn_paths(Some(&config), Some(relative.clone()));
        assert_eq!(resolved, Some(workspace.join(relative)));

        let absolute = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("absolute-service");
        let (_, resolved) = App::resolve_spawn_paths(Some(&config), Some(absolute.clone()));
        assert_eq!(resolved, Some(absolute));
    }

    #[cfg(unix)]
    /// Verifies a launch alias remains independently switchable when its target
    /// is registered, and returning to it restores its workspace directory.
    #[test]
    fn symlinked_launch_alias_survives_switching_to_its_registered_target() {
        use std::{fs, os::unix::fs::symlink};

        let dir = std::env::temp_dir().join(format!("muster-app-link-{}", std::process::id()));
        let workspace_dir = dir.join("workspace");
        let shared_dir = dir.join("shared");
        let target = shared_dir.join(PROJECT_CONFIG_FILE);
        let link = workspace_dir.join(PROJECT_CONFIG_FILE);
        fs::create_dir_all(&workspace_dir).unwrap();
        fs::create_dir_all(&shared_dir).unwrap();
        fs::write(&target, "").unwrap();
        symlink(&target, &link).unwrap();
        let (sender, _receiver) = bounded(16);
        let registry = Box::new(FakeRegistry {
            projects: vec![
                Project::builder()
                    .name(ProjectName::try_new("target").unwrap())
                    .config(target.clone())
                    .build(),
            ],
            workspace: empty_workspace_config(),
            recorder: Recorder::default(),
        });
        let mut app = App::new(
            Workspace::builder().processes(Vec::new()).build(),
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            registry,
            link.clone(),
        );

        app.start();
        assert_eq!(app.projects.len(), 2);
        assert_eq!(
            app.launched_project_membership,
            LaunchedProjectMembership::Synthetic
        );

        app.activate_other_project(0);
        assert_eq!(app.current_config, Some(target));
        app.activate_other_project(0);
        let (project, working_dir) = App::resolve_spawn_paths(app.current_config.as_deref(), None);

        assert_eq!(app.current_config, Some(link.clone()));
        assert_eq!(project, Some(link));
        assert_eq!(working_dir, Some(workspace_dir));
        fs::remove_dir_all(dir).unwrap();
    }

    /// Builds the opt-in shutdown policy used by command lifecycle tests.
    fn test_stop_policy() -> StopPolicy {
        StopPolicy::builder()
            .signal(StopSignal::Interrupt)
            .grace_period(TEST_STOP_GRACE)
            .build()
    }

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
        let stop = (kind == ProcessKind::Command).then(test_stop_policy);
        app_with_stop_signals_and_policy(kind, stop)
    }

    /// Builds a live app with an explicit optional policy and stop-signal record.
    fn app_with_stop_signals_and_policy(
        kind: ProcessKind,
        stop: Option<StopPolicy>,
    ) -> (App, Arc<StopSignals>) {
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
                    .stop(stop)
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
                    .stop((kind == ProcessKind::Command).then(test_stop_policy))
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

    /// Current graceful shutdown request generation for the test pane.
    fn current_shutdown_gen(app: &App) -> ShutdownGeneration {
        app.panes
            .get(&PaneId::new(PANE))
            .unwrap()
            .shutdown_generation
    }

    /// A child that enables xterm mouse tracking receives pointer events instead
    /// of having its terminal text selected by Muster.
    #[test]
    fn mouse_aware_terminal_receives_sgr_pointer_input() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(PANE))
                    .name(ProcessName::try_new("p").unwrap())
                    .kind(ProcessKind::Terminal)
                    .command(Some(CommandLine::try_new("sh").unwrap()))
                    .restart(RestartPolicy::Never)
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
        app.panes
            .get_mut(&PaneId::new(PANE))
            .expect("selected pane exists")
            .parser
            .process(b"\x1b[?1002h\x1b[?1006h");
        let (_, main, _) = areas(app.frame_area);
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: main.x + BORDER_THICKNESS,
            row: main.y + BORDER_THICKNESS,
            modifiers: KeyModifiers::NONE,
        };

        app.handle_input(CrosstermEvent::Mouse(click));

        assert_eq!(*written.lock().unwrap(), b"\x1b[<0;1;1M".to_vec());
    }

    /// Builds a pointer event at an absolute terminal coordinate.
    fn mouse_event(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// Feeds bytes into the selected pane's parser.
    fn feed_pane(app: &mut App, bytes: &[u8]) {
        app.panes
            .get_mut(&PaneId::new(PANE))
            .expect("selected pane exists")
            .parser
            .process(bytes);
    }

    /// Dragging across pane text queues exactly that text for the clipboard.
    #[test]
    fn drag_selection_copies_pane_text() {
        let mut app = app_with(RestartPolicy::Never);
        feed_pane(&mut app, b"hello world");
        let (_, main, _) = areas(app.frame_area);
        let (x, y) = (main.x + BORDER_THICKNESS, main.y + BORDER_THICKNESS);

        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            x,
            y,
        )));
        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Drag(MouseButton::Left),
            x + 4,
            y,
        )));
        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Up(MouseButton::Left),
            x + 4,
            y,
        )));

        assert_eq!(app.take_pending_clipboard(), Some("hello".to_string()));
        assert_eq!(app.take_pending_clipboard(), None);
    }

    /// A bare click never copies anything.
    #[test]
    fn click_without_drag_copies_nothing() {
        let mut app = app_with(RestartPolicy::Never);
        feed_pane(&mut app, b"hello world");
        let (_, main, _) = areas(app.frame_area);
        let (x, y) = (main.x + BORDER_THICKNESS, main.y + BORDER_THICKNESS);

        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            x,
            y,
        )));
        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Up(MouseButton::Left),
            x,
            y,
        )));

        assert_eq!(app.take_pending_clipboard(), None);
    }

    /// A drag that starts over the sidebar selects nothing in the pane.
    #[test]
    fn drag_from_sidebar_selects_nothing() {
        let mut app = app_with(RestartPolicy::Never);
        feed_pane(&mut app, b"hello world");
        let (sidebar, main, _) = areas(app.frame_area);

        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            sidebar.x + 1,
            sidebar.y + 1,
        )));
        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Drag(MouseButton::Left),
            main.x + BORDER_THICKNESS + 5,
            main.y + BORDER_THICKNESS,
        )));
        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Up(MouseButton::Left),
            main.x + BORDER_THICKNESS + 5,
            main.y + BORDER_THICKNESS,
        )));

        assert_eq!(app.take_pending_clipboard(), None);
    }

    /// The wheel moves the primary screen through its scrollback.
    #[test]
    fn wheel_scrolls_primary_screen_scrollback() {
        let mut app = app_with(RestartPolicy::Never);
        for line in 0..64 {
            feed_pane(&mut app, format!("line {line}\r\n").as_bytes());
        }
        let (_, main, _) = areas(app.frame_area);

        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::ScrollUp,
            main.x + BORDER_THICKNESS,
            main.y + BORDER_THICKNESS,
        )));

        let screen = app
            .panes
            .get(&PaneId::new(PANE))
            .expect("selected pane exists")
            .parser
            .screen();
        assert_eq!(screen.scrollback(), WHEEL_SCROLL_LINES);
    }

    /// An alternate-screen child receives cursor keys per wheel notch instead,
    /// matching the host terminal's alternate-scroll behavior.
    #[test]
    fn wheel_in_alternate_screen_sends_cursor_keys() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(PANE))
                    .name(ProcessName::try_new("p").unwrap())
                    .kind(ProcessKind::Terminal)
                    .command(Some(CommandLine::try_new("sh").unwrap()))
                    .restart(RestartPolicy::Never)
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
        feed_pane(&mut app, b"\x1b[?1049h");
        let (_, main, _) = areas(app.frame_area);

        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::ScrollDown,
            main.x + BORDER_THICKNESS,
            main.y + BORDER_THICKNESS,
        )));

        assert_eq!(
            *written.lock().unwrap(),
            b"\x1b[B".repeat(WHEEL_SCROLL_LINES)
        );
    }

    /// Clicking pane text attaches the terminal, like herdr's click-to-focus.
    #[test]
    fn pane_click_focuses_the_terminal() {
        let mut app = app_with(RestartPolicy::Never);
        feed_pane(&mut app, b"hello");
        let (_, main, _) = areas(app.frame_area);

        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            main.x + BORDER_THICKNESS,
            main.y + BORDER_THICKNESS,
        )));

        assert_eq!(app.focus, Focus::Terminal);
    }

    /// A double-click copies the word under the pointer and its confirming
    /// highlight expires on its own.
    #[test]
    fn double_click_copies_the_word_under_the_pointer() {
        let mut app = app_with(RestartPolicy::Never);
        feed_pane(&mut app, b"hello world");
        let (_, main, _) = areas(app.frame_area);
        let (x, y) = (main.x + BORDER_THICKNESS + 7, main.y + BORDER_THICKNESS);
        for _ in 0..2 {
            app.handle_input(CrosstermEvent::Mouse(mouse_event(
                MouseEventKind::Down(MouseButton::Left),
                x,
                y,
            )));
            app.handle_input(CrosstermEvent::Mouse(mouse_event(
                MouseEventKind::Up(MouseButton::Left),
                x,
                y,
            )));
        }

        assert_eq!(app.take_pending_clipboard(), Some("world".to_string()));
        assert!(app.selection.is_some());
        assert!(app.advance_selection(Instant::now() + PANE_COPY_HIGHLIGHT_DURATION));
        assert!(app.selection.is_none());
    }

    /// Dragging above the pane scrolls into scrollback immediately and keeps
    /// scrolling on the autoscroll tick.
    #[test]
    fn drag_past_top_edge_autoscrolls_into_scrollback() {
        let mut app = app_with(RestartPolicy::Never);
        for line in 0..64 {
            feed_pane(&mut app, format!("line {line}\r\n").as_bytes());
        }
        let (_, main, _) = areas(app.frame_area);
        let (x, top) = (main.x + BORDER_THICKNESS, main.y + BORDER_THICKNESS);

        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            x,
            top,
        )));
        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Drag(MouseButton::Left),
            x + 2,
            top,
        )));
        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Drag(MouseButton::Left),
            x + 2,
            main.y,
        )));

        let scrolled = app
            .pane_scroll_metrics(PaneId::new(PANE))
            .expect("pane metrics")
            .offset();
        assert_eq!(scrolled, selection::edge_scroll_lines(1));
        assert!(app.advance_selection(Instant::now() + SELECTION_AUTOSCROLL_INTERVAL));
        let ticked = app
            .pane_scroll_metrics(PaneId::new(PANE))
            .expect("pane metrics")
            .offset();
        assert_eq!(ticked, scrolled + 1);
    }

    /// The wheel keeps extending an in-progress selection while it scrolls.
    #[test]
    fn wheel_extends_an_active_selection() {
        let mut app = app_with(RestartPolicy::Never);
        for line in 0..64 {
            feed_pane(&mut app, format!("line {line}\r\n").as_bytes());
        }
        let (_, main, _) = areas(app.frame_area);
        let (x, y) = (main.x + BORDER_THICKNESS, main.y + BORDER_THICKNESS);

        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            x,
            y,
        )));
        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::ScrollUp,
            x,
            y,
        )));
        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Up(MouseButton::Left),
            x,
            y,
        )));

        let copied = app.take_pending_clipboard().expect("wheel-extended copy");
        assert!(copied.contains("line 41"));
    }

    /// A selection spanning several screens extracts across scrollback chunks
    /// and restores the viewer's scroll position.
    #[test]
    fn extraction_walks_scrollback_chunks() {
        let mut app = app_with(RestartPolicy::Never);
        for line in 0..64 {
            feed_pane(&mut app, format!("line {line}\r\n").as_bytes());
        }
        let mut selection = Selection::anchored(
            PaneId::new(PANE),
            BufferCell::builder().row(0).column(0).build(),
        );
        selection.extend_to(BufferCell::builder().row(45).column(3).build());

        let text = app
            .extract_selection_text(PaneId::new(PANE), &selection)
            .expect("spanning extraction");

        assert!(text.starts_with("line 0"));
        assert!(text.contains("line 30"));
        assert!(text.ends_with("line"));
        let restored = app
            .pane_scroll_metrics(PaneId::new(PANE))
            .expect("pane metrics")
            .offset();
        assert_eq!(restored, 0);
    }

    /// Clicking a sidebar row selects that process, like keyboard navigation.
    #[test]
    fn sidebar_click_selects_the_clicked_process() {
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(PANE))
                    .name(ProcessName::try_new("first").unwrap())
                    .kind(ProcessKind::Terminal)
                    .command(Some(CommandLine::try_new("true").unwrap()))
                    .restart(RestartPolicy::Never)
                    .build(),
                Process::builder()
                    .id(PaneId::new(1))
                    .name(ProcessName::try_new("second").unwrap())
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
            empty_registry(),
            PathBuf::from("muster.yml"),
        );
        app.start();
        let (sidebar_area, ..) = areas(app.frame_area);
        // Rows: project header, section header, then one row per process.
        let second_process_row = sidebar_area.y + 3;

        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            sidebar_area.x + 1,
            second_process_row,
        )));

        assert_eq!(*app.workspace.selected_index(), 1);
        assert_eq!(app.focus, Focus::Sidebar);
    }

    /// Clicking anywhere in the sidebar, even dead space, focuses it back
    /// without disturbing the current selection.
    #[test]
    fn sidebar_click_on_dead_space_refocuses_the_sidebar() {
        let mut app = app_with(RestartPolicy::Never);
        feed_pane(&mut app, b"hello");
        let (sidebar_area, main, _) = areas(app.frame_area);
        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            main.x + BORDER_THICKNESS,
            main.y + BORDER_THICKNESS,
        )));
        assert_eq!(app.focus, Focus::Terminal);

        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            sidebar_area.x + 1,
            sidebar_area.bottom() - 1,
        )));

        assert_eq!(app.focus, Focus::Sidebar);
        assert_eq!(*app.workspace.selected_index(), 0);
    }

    /// Hovering pane text queues the I-beam pointer; leaving it restores the
    /// arrow, and unchanged positions queue nothing.
    #[test]
    fn hovering_pane_text_requests_the_ibeam_pointer() {
        let mut app = app_with(RestartPolicy::Never);
        let (sidebar_area, main, _) = areas(app.frame_area);
        let inside = (main.x + BORDER_THICKNESS, main.y + BORDER_THICKNESS);

        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Moved,
            inside.0,
            inside.1,
        )));
        assert_eq!(app.take_pending_pointer_shape(), Some(PointerShape::Text));

        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Moved,
            inside.0 + 1,
            inside.1,
        )));
        assert_eq!(app.take_pending_pointer_shape(), None);

        app.handle_input(CrosstermEvent::Mouse(mouse_event(
            MouseEventKind::Moved,
            sidebar_area.x + 1,
            sidebar_area.y + 1,
        )));
        assert_eq!(
            app.take_pending_pointer_shape(),
            Some(PointerShape::Default)
        );
    }

    /// A failed first PTY spawn keeps a reported-ID session safely retryable.
    #[test]
    fn failed_initial_agent_spawn_remains_a_pending_fresh_launch() {
        let (sender, _receiver) = bounded(16);
        let sessions = SessionRecorder::default();
        let mut app = App::new(
            Workspace::builder().processes(Vec::new()).build(),
            Box::new(FailingRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            empty_registry(),
            PathBuf::from("muster.yml"),
        );
        app.set_agent_session_store(Box::new(FakeAgentSessionStore {
            recorder: sessions.clone(),
        }));
        app.start();

        app.create_agent_session(AgentTool::Codex, None, None, None);

        let pane = *app
            .workspace
            .selected_process()
            .expect("agent pane exists")
            .id();
        let session = sessions
            .sessions
            .borrow()
            .last()
            .cloned()
            .expect("session persisted");
        assert_eq!(
            *app.workspace.process(pane).expect("agent exists").state(),
            ProcessState::Crashed
        );
        assert_eq!(*session.state(), AgentSessionState::Pending);
        assert_eq!(
            session
                .restore_command()
                .expect("fresh retry command")
                .as_ref(),
            "codex"
        );
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
    fn a_foreground_terminal_bell_marks_awaiting_input_without_notifying() {
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
        assert!(
            app.notice.is_none(),
            "the sidebar marker is sufficient in the foreground"
        );
        assert_eq!(notifier.count(), 0, "foreground shell bells stay local");
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
    fn a_burst_of_foreground_bells_does_not_notify() {
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
        assert_eq!(notifier.count(), 0, "foreground shell bells stay local");
    }

    /// A quiet background terminal bell escalates after a short grace period.
    #[test]
    fn a_quiet_background_terminal_bell_escalates_once() {
        let mut app = app_with(RestartPolicy::Never);
        let notifier = RecordingNotifier::default();
        app.set_notifier(Box::new(notifier.clone()));
        enable_desktop(&mut app);
        let background = PaneId::new(PANE + 1);
        app.workspace.insert_in_section(
            Process::builder()
                .id(background)
                .name(ProcessName::try_new("background").unwrap())
                .kind(ProcessKind::Terminal)
                .command(Some(CommandLine::try_new("sh").unwrap()))
                .restart(RestartPolicy::Never)
                .build(),
        );
        app.spawn(background, Some(CommandLine::try_new("sh").unwrap()), None);
        let generation = *app
            .generations
            .get(&background)
            .expect("background pane spawned");

        app.handle_output(
            background,
            generation,
            ProcessOutput::Chunk(b"\x07".to_vec()),
        );

        assert_eq!(notifier.count(), 0, "the bell is initially sidebar-only");
        let deadline = app
            .next_activity_deadline()
            .expect("background bell has an escalation deadline");
        assert!(app.expire_quiet_activity(deadline));

        assert_eq!(notifier.bodies(), [Some(AWAITING_INPUT_NOTICE.to_string())]);
    }

    /// A terminal that exits during the bell grace period must not raise a
    /// late desktop alert for a process that can no longer need input.
    #[test]
    fn an_exited_background_terminal_cancels_its_pending_bell() {
        let mut app = app_with(RestartPolicy::Never);
        let notifier = RecordingNotifier::default();
        app.set_notifier(Box::new(notifier.clone()));
        enable_desktop(&mut app);
        let background = PaneId::new(PANE + 1);
        app.workspace.insert_in_section(
            Process::builder()
                .id(background)
                .name(ProcessName::try_new("background").unwrap())
                .kind(ProcessKind::Terminal)
                .command(Some(CommandLine::try_new("sh").unwrap()))
                .restart(RestartPolicy::Never)
                .build(),
        );
        app.spawn(background, Some(CommandLine::try_new("sh").unwrap()), None);
        let generation = *app
            .generations
            .get(&background)
            .expect("background pane spawned");

        app.handle_output(
            background,
            generation,
            ProcessOutput::Chunk(b"\x07".to_vec()),
        );
        app.handle_output(
            background,
            generation,
            ProcessOutput::Exited(ExitOutcome::Succeeded),
        );

        assert!(!app.expire_quiet_activity(Instant::now() + BELL_ESCALATION_DELAY));
        assert_eq!(notifier.count(), 0);
    }

    /// Agent bells remain immediate because they are part of the agent attention contract.
    #[test]
    fn an_agent_bell_notifies_immediately() {
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![
                Process::builder()
                    .id(PaneId::new(PANE))
                    .name(ProcessName::try_new("agent").unwrap())
                    .kind(ProcessKind::Agent)
                    .command(Some(CommandLine::try_new("agent").unwrap()))
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
            empty_registry(),
            PathBuf::from("muster.yml"),
        );
        let notifier = RecordingNotifier::default();
        app.set_notifier(Box::new(notifier.clone()));
        app.set_settings_store(Box::new(FakeSettingsStore::default()));
        app.start();

        app.handle_output(
            PaneId::new(PANE),
            current_gen(&app),
            ProcessOutput::Chunk(b"\x07".to_vec()),
        );

        assert_eq!(notifier.bodies(), [Some(AWAITING_INPUT_NOTICE.to_string())]);
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
            ProcessOutput::Chunk(b"\x1b]9;attention requested\x07".to_vec()),
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

    /// A configured stop sends its signal once, enters stopping, and escalates.
    #[test]
    fn repeated_manual_command_stop_reuses_one_graceful_escalation() {
        let (mut app, signals) = app_with_stop_signals(ProcessKind::Command);
        app.stop_selected();
        app.stop_selected();

        assert_eq!(signals.terminates.load(Ordering::SeqCst), 1);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 0);
        assert_eq!(
            *signals.termination.lock().unwrap(),
            Some((StopSignal::Interrupt, TEST_STOP_GRACE))
        );
        assert_eq!(state(&app), ProcessState::Stopping);

        app.handle_force_stop(
            PaneId::new(PANE),
            current_gen(&app),
            current_shutdown_gen(&app),
        );
        assert_eq!(signals.kills.load(Ordering::SeqCst), 1);
        assert_eq!(app.notice.as_deref(), Some(FORCE_STOP_NOTICE));
    }

    /// Commands use the default graceful policy when `stop` is absent.
    #[test]
    fn a_command_without_a_stop_policy_uses_graceful_defaults() {
        let (mut app, signals) = app_with_stop_signals_and_policy(ProcessKind::Command, None);

        app.stop_selected();

        assert_eq!(signals.terminates.load(Ordering::SeqCst), 1);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 0);
        let policy = StopPolicy::default();
        assert_eq!(
            *signals.termination.lock().unwrap(),
            Some((*policy.signal(), *policy.grace_period()))
        );
        assert_eq!(state(&app), ProcessState::Stopping);
    }

    /// Force-stop ignores a command's configured graceful shutdown path.
    #[test]
    fn force_stop_bypasses_a_commands_graceful_policy() {
        let (mut app, signals) = app_with_stop_signals(ProcessKind::Command);

        app.force_stop_selected();

        assert_eq!(signals.terminates.load(Ordering::SeqCst), 0);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 1);
        assert_eq!(state(&app), ProcessState::Stopping);
    }

    /// Restart waits for configured graceful exit before spawning a new child.
    #[test]
    fn restart_uses_a_commands_graceful_policy_then_respawns() {
        let (mut app, signals) = app_with_stop_signals(ProcessKind::Command);
        let pane = PaneId::new(PANE);

        app.restart_selected();

        assert_eq!(signals.terminates.load(Ordering::SeqCst), 1);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 0);
        assert_eq!(state(&app), ProcessState::Stopping);
        assert_eq!(
            app.panes.get(&pane).unwrap().exit_intent,
            ExitIntent::RestartInFlight
        );

        app.handle_output(
            pane,
            current_gen(&app),
            ProcessOutput::Exited(ExitOutcome::Succeeded),
        );

        assert_eq!(state(&app), ProcessState::Running);
        assert!(app.panes.get(&pane).unwrap().handle.is_some());
    }

    /// Superseding a graceful stop gives the replacement restart its full grace
    /// period by making the earlier escalation event stale.
    #[test]
    fn a_new_graceful_action_invalidates_the_previous_deadline() {
        let (mut app, signals) = app_with_stop_signals(ProcessKind::Command);
        let pane = PaneId::new(PANE);
        let spawn_generation = current_gen(&app);

        app.stop_selected();
        let stale_shutdown = current_shutdown_gen(&app);
        app.restart_selected();
        let current_shutdown = current_shutdown_gen(&app);

        assert_ne!(stale_shutdown, current_shutdown);
        app.handle_force_stop(pane, spawn_generation, stale_shutdown);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 0);
        assert_eq!(
            app.panes.get(&pane).unwrap().exit_intent,
            ExitIntent::RestartInFlight
        );

        app.handle_force_stop(pane, spawn_generation, current_shutdown);
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
        app.handle_force_stop(pane, current_gen(&app), current_shutdown_gen(&app));
        assert_eq!(
            app.panes.get(&pane).unwrap().exit_intent,
            ExitIntent::StopRetryable
        );

        app.stop_selected();
        app.handle_force_stop(pane, current_gen(&app), current_shutdown_gen(&app));

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

    /// Builds an empty app wired to observable durable session state and PTY
    /// requests, then runs startup restoration.
    fn agent_app(initial: Vec<AgentSession>) -> (App, SessionRecorder, SpawnRecorder) {
        let sessions = SessionRecorder {
            sessions: Rc::new(RefCell::new(initial)),
        };
        let spawns = SpawnRecorder::default();
        let (sender, _receiver) = bounded(16);
        let mut app = App::new(
            Workspace::builder().processes(Vec::new()).build(),
            Box::new(RequestRecordingRunner {
                recorder: spawns.clone(),
            }),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            empty_registry(),
            PathBuf::from("/here/muster.yml"),
        );
        app.set_agent_session_store(Box::new(FakeAgentSessionStore {
            recorder: sessions.clone(),
        }));
        app.start();
        (app, sessions, spawns)
    }

    /// Builds a durable provider session for restoration and history tests.
    fn persisted_agent_session(
        tool: AgentTool,
        native_id: &str,
        state: AgentSessionState,
    ) -> AgentSession {
        AgentSession::builder()
            .id(AgentSessionId::generate().unwrap())
            .name(ProcessName::try_new("Ada").unwrap())
            .tool(tool)
            .project(PathBuf::from("/here/muster.yml"))
            .launch_command(CommandLine::try_new(tool.default_command().unwrap()).unwrap())
            .native_id(Some(NativeSessionId::try_new(native_id).unwrap()))
            .state(state)
            .build()
    }

    /// Builds an app whose in-memory terminal has not yet observed its removal
    /// from the empty workspace config on disk.
    fn stale_terminal_app() -> App {
        let (sender, _receiver) = bounded(16);
        let stale = ProcessSpec::builder()
            .name(ProcessName::try_new("stale").unwrap())
            .command(Some(CommandLine::try_new("stale-command").unwrap()))
            .build()
            .to_process(PaneId::new(PANE), ProcessKind::Terminal);
        App::new(
            Workspace::builder().processes(vec![stale]).build(),
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            Box::new(FakeRegistry {
                projects: vec![],
                workspace: empty_workspace_config(),
                recorder: Recorder::default(),
            }),
            PathBuf::from("/here/muster.yml"),
        )
    }

    /// Builds an app whose live model omits `stop` while the reloaded config
    /// writes the equivalent default policy explicitly.
    fn equivalent_stop_reconciliation_app() -> App {
        let implicit = ProcessSpec::builder()
            .name(ProcessName::try_new("server").unwrap())
            .command(Some(CommandLine::try_new("serve").unwrap()))
            .build();
        let explicit = implicit.clone().with_stop(Some(StopPolicy::default()));
        let loaded = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![implicit])
            .build();
        let updated = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![explicit])
            .build();
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(loaded.to_processes())
            .build();
        App::new(
            workspace,
            Box::new(FakeRunner),
            sender,
            Rect::new(0, 0, 80, 24),
            Box::new(FakeCompleter),
            Box::new(FakeRegistry {
                projects: vec![],
                workspace: updated,
                recorder: Recorder::default(),
            }),
            PathBuf::from("/here/muster.yml"),
        )
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

    /// Writing the implicit stop defaults explicitly does not replace a stopped
    /// pane because both configurations have the same effective behavior.
    #[test]
    fn reconciliation_normalizes_equivalent_stop_defaults_for_a_stopped_command() {
        let mut app = equivalent_stop_reconciliation_app();
        let original_pane = *app.workspace.processes()[0].id();

        app.handle_config_changed(PathBuf::from("/here/muster.yml"));

        assert_eq!(app.workspace.processes().len(), 1);
        assert_eq!(*app.workspace.processes()[0].id(), original_pane);
    }

    /// Writing the implicit stop defaults explicitly neither retires nor
    /// duplicates a running command.
    #[test]
    fn reconciliation_normalizes_equivalent_stop_defaults_for_a_running_command() {
        let mut app = equivalent_stop_reconciliation_app();
        let pane = *app.workspace.processes()[0].id();
        app.start_selected();

        app.handle_config_changed(PathBuf::from("/here/muster.yml"));

        assert_eq!(app.workspace.processes().len(), 1);
        assert_eq!(
            app.panes.get(&pane).unwrap().config_membership,
            ConfigMembership::Tracked
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

    #[cfg(unix)]
    /// Verifies registering and removing one symlink location does not mutate a
    /// second registration whose alias happens to share the same target.
    #[test]
    fn registry_mutations_preserve_distinct_symlink_locations() {
        use std::{fs, os::unix::fs::symlink};

        let dir = std::env::temp_dir().join(format!("muster-mutation-link-{}", std::process::id()));
        let shared_dir = dir.join("shared");
        let first_dir = dir.join("first");
        let second_dir = dir.join("second");
        let target = shared_dir.join(PROJECT_CONFIG_FILE);
        let first = first_dir.join(PROJECT_CONFIG_FILE);
        let second = second_dir.join(PROJECT_CONFIG_FILE);
        fs::create_dir_all(&shared_dir).unwrap();
        fs::create_dir_all(&first_dir).unwrap();
        fs::create_dir_all(&second_dir).unwrap();
        fs::write(&target, "").unwrap();
        symlink(&target, &first).unwrap();
        symlink(&target, &second).unwrap();
        let projects = vec![
            Project::builder()
                .name(ProjectName::try_new("first").unwrap())
                .config(first.clone())
                .build(),
            Project::builder()
                .name(ProjectName::try_new("second").unwrap())
                .config(second.clone())
                .build(),
        ];
        let (mut app, recorder) =
            flow_app(projects, empty_workspace_config(), "/unrelated/muster.yml");
        let renamed = Project::builder()
            .name(ProjectName::try_new("renamed").unwrap())
            .config(first.clone())
            .build();

        assert!(app.try_register(renamed));
        let saved = recorder.projects.borrow().clone().unwrap();
        assert_eq!(saved.len(), 2);
        assert!(saved.iter().any(|project| project.config() == &second));

        app.remove_project(&first);
        let saved = recorder.projects.borrow().clone().unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].config(), &second);

        fs::remove_dir_all(dir).unwrap();
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

    /// Verifies unsupported relative registry entries report an error rather than
    /// loading against the TUI process's current directory from either UI path.
    #[test]
    fn relative_projects_are_not_opened_from_the_ui() {
        let projects = vec![project("legacy", PROJECT_CONFIG_FILE)];
        let (mut app, _recorder) = flow_app(projects, empty_workspace_config(), "/here/muster.yml");

        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);

        assert!(app.pending_switch.is_none());
        assert!(
            app.notice
                .as_deref()
                .is_some_and(|notice| notice.contains("unsupported relative"))
        );

        app.open_switcher();
        app.handle_switcher_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(app.pending_switch.is_none());
        assert!(app.switcher().unwrap().error.is_some());
        assert_eq!(app.current_config, Some(PathBuf::from("/here/muster.yml")));
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
                    .stop(Some(test_stop_policy()))
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
        assert_eq!(
            workspaces[0].1.commands()[0].stop(),
            &Some(test_stop_policy()),
            "changing autostart preserves the independent stop policy"
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

    /// Verifies saving a relatively launched workspace registers its absolute
    /// config path and closes the form.
    #[test]
    fn saving_the_current_workspace_registers_it() {
        let (mut app, recorder) = flow_app(vec![], empty_workspace_config(), PROJECT_CONFIG_FILE);
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
        assert_eq!(
            saved[0].config(),
            &path::absolutize(Path::new(PROJECT_CONFIG_FILE))
        );
        assert!(saved[0].config().is_absolute());
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
        app.add_configured_process(ProcessKind::Terminal, &[
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

    /// Adding persistent state reconciles in place and leaves a live session
    /// attached to the same child generation.
    #[test]
    fn adding_a_configured_process_preserves_agent_sessions() {
        let (mut app, _recorder) = flow_app(vec![], empty_workspace_config(), "/here/muster.yml");
        app.launch_agent_session(&["Codex".to_string(), String::new(), String::new()]);
        let session = *app.workspace.selected_process().unwrap().id();
        let generation = app.generations[&session];

        app.add_configured_process(ProcessKind::Terminal, &[
            "logs".to_string(),
            "tail -f log".to_string(),
        ]);

        let process = app.workspace.process(session).unwrap();
        assert_eq!(*process.origin(), ProcessOrigin::Session);
        assert_eq!(app.generations[&session], generation);
        assert!(app.panes[&session].handle.is_some());
        assert!(app.pending_switch.is_none());
        assert_eq!(*app.workspace.selected_process().unwrap().id(), session);
        let added = app
            .workspace
            .processes()
            .iter()
            .find(|process| process.name().as_ref() == "logs")
            .unwrap();
        assert_eq!(*added.origin(), ProcessOrigin::Configured);
        assert_eq!(*added.state(), ProcessState::Running);
    }

    /// A form submission must not autostart an unrelated process that appeared
    /// on disk before the locked append read the workspace.
    #[test]
    fn adding_a_process_does_not_start_an_unreconciled_disk_addition() {
        let external = ProcessSpec::builder()
            .name(ProcessName::try_new("external").unwrap())
            .command(Some(CommandLine::try_new("external-command").unwrap()))
            .build();
        let config = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![external])
            .commands(vec![])
            .build();
        let (mut app, _recorder) = flow_app(vec![], config, "/here/muster.yml");

        app.add_configured_process(ProcessKind::Terminal, &[
            "form".to_string(),
            "form-command".to_string(),
        ]);

        let state_of = |name: &str| {
            *app.workspace
                .processes()
                .iter()
                .find(|process| process.name().as_ref() == name)
                .unwrap()
                .state()
        };
        assert_eq!(state_of("external"), ProcessState::Pending);
        assert_eq!(state_of("form"), ProcessState::Running);
    }

    /// Identical concurrent additions are separate occurrences, and only the
    /// occurrence appended by the form is started.
    #[test]
    fn adding_a_process_starts_only_its_identical_spec_occurrence() {
        let external = ProcessSpec::builder()
            .name(ProcessName::try_new("duplicate").unwrap())
            .command(Some(CommandLine::try_new("same-command").unwrap()))
            .build();
        let config = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![external])
            .commands(vec![])
            .build();
        let (mut app, _recorder) = flow_app(vec![], config, "/here/muster.yml");

        app.add_configured_process(ProcessKind::Terminal, &[
            "duplicate".to_string(),
            "same-command".to_string(),
        ]);

        let states = app
            .workspace
            .processes()
            .iter()
            .filter(|process| process.name().as_ref() == "duplicate")
            .map(|process| *process.state())
            .collect::<Vec<_>>();
        assert_eq!(states, vec![ProcessState::Pending, ProcessState::Running]);
    }

    /// An unreconciled modification gets a pending replacement row while the
    /// form's newly appended process is the only row started.
    #[test]
    fn adding_a_process_does_not_start_an_unreconciled_replacement() {
        let replacement = ProcessSpec::builder()
            .name(ProcessName::try_new("p").unwrap())
            .command(Some(CommandLine::try_new("replacement-command").unwrap()))
            .build();
        let config = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![replacement])
            .commands(vec![])
            .build();
        let (mut app, _recorder) = flow_app(vec![], config, "/here/muster.yml");

        app.add_configured_process(ProcessKind::Terminal, &[
            "form".to_string(),
            "form-command".to_string(),
        ]);

        let replacement = app
            .workspace
            .processes()
            .iter()
            .find(|process| {
                process.command().as_ref().map(CommandLine::as_ref) == Some("replacement-command")
            })
            .unwrap();
        let added = app
            .workspace
            .processes()
            .iter()
            .find(|process| process.name().as_ref() == "form")
            .unwrap();
        assert_eq!(*replacement.state(), ProcessState::Pending);
        assert_eq!(*added.state(), ProcessState::Running);
    }

    /// Re-adding a disk-removed terminal reuses and starts its stopped pane when
    /// the watcher has not reconciled the removal yet.
    #[test]
    fn readding_a_stale_stopped_terminal_starts_the_reused_pane() {
        let mut app = stale_terminal_app();
        app.start();
        let generation = current_gen(&app);
        app.handle_output(
            PaneId::new(PANE),
            generation,
            ProcessOutput::Exited(ExitOutcome::Succeeded),
        );
        assert_eq!(state(&app), ProcessState::Exited);

        app.add_configured_process(ProcessKind::Terminal, &[
            "stale".to_string(),
            "stale-command".to_string(),
        ]);

        assert_eq!(app.workspace.processes().len(), 1);
        assert_eq!(state(&app), ProcessState::Running);
        assert_ne!(current_gen(&app), generation);
    }

    /// Re-adding a disk-removed terminal that is still live reuses it without
    /// spawning a duplicate generation.
    #[test]
    fn readding_a_stale_live_terminal_does_not_respawn_it() {
        let mut app = stale_terminal_app();
        app.start();
        let generation = current_gen(&app);

        app.add_configured_process(ProcessKind::Terminal, &[
            "stale".to_string(),
            "stale-command".to_string(),
        ]);

        assert_eq!(app.workspace.processes().len(), 1);
        assert_eq!(state(&app), ProcessState::Running);
        assert_eq!(current_gen(&app), generation);
    }

    /// Agent sessions launch live without persisting a process specification.
    #[test]
    fn launching_an_agent_session_does_not_write_the_config() {
        let (mut app, recorder) = flow_app(vec![], empty_workspace_config(), "/here/muster.yml");

        app.launch_agent_session(&["Codex".to_string(), String::new(), String::new()]);

        let session = app.workspace.selected_process().unwrap();
        assert_eq!(*session.kind(), ProcessKind::Agent);
        assert_eq!(*session.origin(), ProcessOrigin::Session);
        assert_eq!(*session.agent_tool(), Some(AgentTool::Codex));
        assert!(!session.name().as_ref().is_empty());
        assert_eq!(session.command().as_ref().unwrap().as_ref(), "codex");
        assert!(session.agent_session_id().is_some());
        assert!(*session.autostart());
        assert_eq!(app.focus, Focus::Terminal);
        assert!(recorder.workspaces.borrow().is_empty());
    }

    /// New sessions are persisted before spawn and correlate provider hooks
    /// through an environment variable independent of their display name.
    #[test]
    fn launching_an_agent_session_persists_identity_and_exports_correlation() {
        let (mut app, sessions, spawns) = agent_app(Vec::new());

        app.create_agent_session(AgentTool::Claude, None, None, None);

        let sessions = sessions.sessions.borrow();
        let session = sessions.first().unwrap();
        assert_eq!(*session.state(), AgentSessionState::Open);
        assert_eq!(session.project(), Path::new("/here/muster.yml"));
        assert!(session.native_id().is_none());
        let requests = spawns.requests.borrow();
        let request = requests.last().unwrap();
        assert_eq!(
            request
                .environment()
                .get(std::ffi::OsStr::new(MUSTER_AGENT_SESSION_ENV)),
            Some(&OsString::from(session.id().as_ref()))
        );
        assert_eq!(
            request.command().as_ref().unwrap().as_ref(),
            format!("claude --session-id {}", session.id()).as_str()
        );
    }

    /// A working agent schedules and advances the sidebar spinner independently
    /// of process-output redraws.
    #[test]
    fn working_agents_schedule_sidebar_animation() {
        let (mut app, _sessions, _spawns) = agent_app(Vec::new());
        app.create_agent_session(AgentTool::Claude, None, None, None);
        let pane = *app.workspace.selected_process().unwrap().id();
        app.workspace.set_activity(pane, ActivityState::Working);
        let before = app.activity_frame;
        let deadline = app.next_activity_frame_deadline().unwrap();

        assert!(app.advance_activity_frame(deadline));
        assert_ne!(app.activity_frame, before);
        assert!(app.next_activity_frame_deadline().unwrap() > deadline);
    }

    /// Working command output does not schedule agent-specific animation.
    #[test]
    fn working_commands_do_not_schedule_sidebar_animation() {
        let mut app = app_with(RestartPolicy::Never);
        let pane = PaneId::new(PANE);
        app.workspace.set_activity(pane, ActivityState::Working);

        assert!(app.next_activity_frame_deadline().is_none());
    }

    /// Pausing an explicitly working agent suspends its animation deadline
    /// without discarding the provider's activity state.
    #[test]
    fn paused_working_agents_do_not_schedule_animation() {
        let (mut app, _sessions, _spawns) = agent_app(Vec::new());
        app.create_agent_session(AgentTool::Claude, None, None, None);
        let pane = *app.workspace.selected_process().unwrap().id();
        app.workspace.set_activity(pane, ActivityState::Working);

        app.toggle_pause_selected();

        assert_eq!(
            *app.workspace.process(pane).unwrap().state(),
            ProcessState::Paused
        );
        assert_eq!(
            *app.workspace.process(pane).unwrap().activity(),
            ActivityState::Working
        );
        assert!(app.next_activity_frame_deadline().is_none());
    }

    /// Closing a captured session moves it into history, and reopening uses the
    /// provider's native ID without duplicating its Muster identity.
    #[test]
    fn a_closed_agent_session_reopens_the_same_native_conversation() {
        let (mut app, sessions, spawns) = agent_app(Vec::new());
        app.create_agent_session(AgentTool::Codex, None, None, None);
        let pane = *app.workspace.selected_process().unwrap().id();
        let generation = app.generations[&pane];
        let id = app
            .workspace
            .selected_process()
            .unwrap()
            .agent_session_id()
            .clone()
            .unwrap();
        FakeAgentSessionStore {
            recorder: sessions.clone(),
        }
        .set_owner_process_id(&id, AgentProcessId::try_new(1).unwrap(), None, None)
        .unwrap();
        FakeAgentSessionStore {
            recorder: sessions.clone(),
        }
        .capture_native_id(
            &id,
            AgentTool::Codex,
            AgentProcessId::try_new(1).unwrap(),
            None,
            NativeSessionId::try_new("codex-thread").unwrap(),
        )
        .unwrap();

        app.close_agent_session(pane);
        app.handle_output(
            pane,
            generation,
            ProcessOutput::Exited(ExitOutcome::Succeeded),
        );
        assert_eq!(
            *sessions.sessions.borrow().last().unwrap().state(),
            AgentSessionState::Closed
        );

        app.reopen_last_closed_session();

        assert_eq!(
            *sessions.sessions.borrow().last().unwrap().state(),
            AgentSessionState::Open
        );
        assert_eq!(
            app.workspace
                .processes()
                .iter()
                .filter(|process| process.agent_session_id().as_ref() == Some(&id))
                .count(),
            1
        );
        assert_eq!(
            spawns
                .requests
                .borrow()
                .last()
                .unwrap()
                .command()
                .as_ref()
                .unwrap()
                .as_ref(),
            "codex resume codex-thread"
        );
        assert_eq!(app.focus, Focus::Terminal);
    }

    /// Reopening during a delayed close waits for the retiring child before it
    /// creates the replacement pane, so the persisted open state stays visible.
    #[test]
    fn reopening_a_closing_session_respawns_after_its_exit() {
        let (mut app, sessions, spawns) = agent_app(Vec::new());
        app.create_agent_session(AgentTool::Codex, None, None, None);
        let pane = *app.workspace.selected_process().unwrap().id();
        let generation = app.generations[&pane];
        let id = app
            .workspace
            .selected_process()
            .unwrap()
            .agent_session_id()
            .clone()
            .unwrap();
        FakeAgentSessionStore {
            recorder: sessions.clone(),
        }
        .set_owner_process_id(&id, AgentProcessId::try_new(1).unwrap(), None, None)
        .unwrap();
        FakeAgentSessionStore {
            recorder: sessions.clone(),
        }
        .capture_native_id(
            &id,
            AgentTool::Codex,
            AgentProcessId::try_new(1).unwrap(),
            None,
            NativeSessionId::try_new("codex-thread").unwrap(),
        )
        .unwrap();

        app.close_agent_session(pane);
        app.reopen_last_closed_session();

        assert_eq!(spawns.requests.borrow().len(), 1);
        assert_eq!(
            *sessions.sessions.borrow().last().unwrap().state(),
            AgentSessionState::Open
        );
        assert_eq!(app.pending_session_reopens.get(&pane), Some(&id));

        app.handle_output(
            pane,
            generation,
            ProcessOutput::Exited(ExitOutcome::Succeeded),
        );

        assert_eq!(spawns.requests.borrow().len(), 2);
        assert!(app.pending_session_reopens.is_empty());
        assert_eq!(
            app.workspace
                .selected_process()
                .unwrap()
                .agent_session_id()
                .as_ref(),
            Some(&id)
        );
        assert_eq!(app.focus, Focus::Terminal);
    }

    /// Closing a session again cancels a reopen queued while its child exits.
    #[test]
    fn closing_again_cancels_a_queued_session_reopen() {
        let (mut app, sessions, spawns) = agent_app(Vec::new());
        app.create_agent_session(AgentTool::Codex, None, None, None);
        let pane = *app.workspace.selected_process().unwrap().id();
        let generation = app.generations[&pane];
        let id = app
            .workspace
            .selected_process()
            .unwrap()
            .agent_session_id()
            .clone()
            .unwrap();
        let store = FakeAgentSessionStore {
            recorder: sessions.clone(),
        };
        store
            .set_owner_process_id(&id, AgentProcessId::try_new(1).unwrap(), None, None)
            .unwrap();
        store
            .capture_native_id(
                &id,
                AgentTool::Codex,
                AgentProcessId::try_new(1).unwrap(),
                None,
                NativeSessionId::try_new("codex-thread").unwrap(),
            )
            .unwrap();

        app.close_agent_session(pane);
        app.reopen_last_closed_session();
        assert_eq!(app.pending_session_reopens.get(&pane), Some(&id));

        app.close_agent_session(pane);
        app.handle_output(
            pane,
            generation,
            ProcessOutput::Exited(ExitOutcome::Succeeded),
        );

        assert_eq!(spawns.requests.borrow().len(), 1);
        assert!(app.pending_session_reopens.is_empty());
        assert_eq!(
            *sessions.sessions.borrow().last().unwrap().state(),
            AgentSessionState::Closed
        );
    }

    /// Session reopens wait for project switching to finish so a pane cannot be
    /// created in the old workspace and immediately torn down.
    #[test]
    fn reopening_is_rejected_while_a_project_switch_is_pending() {
        let session =
            persisted_agent_session(AgentTool::Codex, "codex-thread", AgentSessionState::Closed);
        let (mut app, _sessions, spawns) = agent_app(vec![session]);
        app.create_agent_session(AgentTool::Claude, None, None, None);
        let spawned = spawns.requests.borrow().len();

        app.begin_switch(empty_workspace_config(), PathBuf::from("next.yml"));
        assert!(app.pending_switch.is_some());

        app.reopen_last_closed_session();

        assert_eq!(spawns.requests.borrow().len(), spawned);
        assert_eq!(app.notice.as_deref(), Some(PROJECT_SWITCH_IN_PROGRESS));
    }

    /// Restart waits for a reported native ID rather than destroying the live
    /// conversation and accidentally launching a new one.
    #[test]
    fn an_uncaptured_agent_session_is_not_restarted_as_a_new_conversation() {
        let (mut app, _sessions, spawns) = agent_app(Vec::new());
        app.create_agent_session(AgentTool::Codex, None, None, None);
        let pane = *app.workspace.selected_process().unwrap().id();
        let generation = app.generations[&pane];

        app.restart_selected();

        assert_eq!(
            *app.workspace.process(pane).unwrap().state(),
            ProcessState::Running
        );
        assert_eq!(app.generations[&pane], generation);
        assert_eq!(spawns.requests.borrow().len(), 1);
        assert_eq!(app.notice.as_deref(), Some(AGENT_SESSION_NOT_RESUMABLE));
    }

    /// A store read failure after exit settles the pane instead of leaving it restarting.
    #[test]
    fn restart_after_exit_surfaces_a_session_store_failure() {
        let (mut app, _sessions, _spawns) = agent_app(Vec::new());
        app.create_agent_session(AgentTool::Claude, None, None, None);
        let pane = *app
            .workspace
            .selected_process()
            .expect("agent pane exists")
            .id();
        let generation = app.generations[&pane];
        app.panes
            .get_mut(&pane)
            .expect("live pane exists")
            .exit_intent = ExitIntent::RestartInFlight;
        app.set_agent_session_store(Box::new(UnreadableAgentSessionStore));

        app.handle_output(pane, generation, ProcessOutput::Exited(ExitOutcome::Failed));

        assert_eq!(
            *app.workspace
                .process(pane)
                .expect("agent remains visible")
                .state(),
            ProcessState::Crashed
        );
        assert!(
            app.notice
                .as_deref()
                .is_some_and(|notice| notice.starts_with(AGENT_SESSION_STORE_ERROR))
        );
    }

    /// Reopen skips newer history that lacks a provider identity instead of
    /// making an older resumable conversation unreachable from the hotkey.
    #[test]
    fn reopen_uses_the_latest_resumable_agent_session() {
        let older = persisted_agent_session(
            AgentTool::Codex,
            "captured-thread",
            AgentSessionState::Closed,
        );
        let older_id = older.id().clone();
        let newer = AgentSession::builder()
            .id(AgentSessionId::generate().unwrap())
            .name(ProcessName::try_new("Grace").unwrap())
            .tool(AgentTool::Gemini)
            .project(PathBuf::from("/here/muster.yml"))
            .launch_command(CommandLine::try_new("gemini").unwrap())
            .state(AgentSessionState::Closed)
            .build();
        let (mut app, _sessions, spawns) = agent_app(vec![older, newer]);

        app.reopen_last_closed_session();

        assert_eq!(
            app.workspace.processes()[0].agent_session_id().as_ref(),
            Some(&older_id)
        );
        assert_eq!(
            spawns.requests.borrow()[0]
                .command()
                .as_ref()
                .unwrap()
                .as_ref(),
            "codex resume captured-thread"
        );
    }

    /// Startup restores open sessions with their resume command but leaves
    /// closed history dormant.
    #[test]
    fn startup_restores_only_open_agent_sessions() {
        let open =
            persisted_agent_session(AgentTool::Gemini, "gemini-open", AgentSessionState::Open);
        let open_id = open.id().clone();
        let closed = persisted_agent_session(
            AgentTool::Copilot,
            "copilot-closed",
            AgentSessionState::Closed,
        );

        let (app, _sessions, spawns) = agent_app(vec![open, closed]);

        assert_eq!(app.workspace.processes().len(), 1);
        assert_eq!(
            app.workspace.processes()[0].agent_session_id().as_ref(),
            Some(&open_id)
        );
        assert_eq!(
            spawns.requests.borrow()[0]
                .command()
                .as_ref()
                .unwrap()
                .as_ref(),
            "gemini --resume gemini-open"
        );
        assert!(*app.workspace.processes()[0].autostart());
        assert_eq!(app.focus, Focus::Sidebar);
    }

    /// A second Muster instance leaves a session alone while its owner lives.
    #[cfg(unix)]
    #[test]
    fn startup_skips_an_open_session_with_a_live_owner() {
        let owner = AgentProcessId::try_new(std::process::id()).unwrap();
        let token = LocalProcessIdentity::start_token(owner).unwrap();
        let session =
            persisted_agent_session(AgentTool::Codex, "owned-thread", AgentSessionState::Open)
                .with_launch_processes(owner, Some(token), None);

        let (app, _sessions, spawns) = agent_app(vec![session]);

        assert!(app.workspace.processes().is_empty());
        assert!(spawns.requests.borrow().is_empty());
    }

    /// A numeric PID without its creation token cannot suppress restoration,
    /// because it may already belong to an unrelated process.
    #[test]
    fn startup_restores_a_session_with_an_unverifiable_owner() {
        let owner = AgentProcessId::try_new(std::process::id()).unwrap();
        let session = persisted_agent_session(
            AgentTool::Codex,
            "unverifiable-owner-thread",
            AgentSessionState::Open,
        )
        .with_owner_process_id(owner);

        let (app, _sessions, spawns) = agent_app(vec![session]);

        assert_eq!(app.workspace.processes().len(), 1);
        assert_eq!(spawns.requests.borrow().len(), 1);
    }

    /// An unconfirmed caller-assigned identity retries its original launch
    /// command instead of resuming a conversation the provider never created.
    #[test]
    fn startup_retries_an_unconfirmed_assigned_agent_session() {
        let session = AgentSession::builder()
            .id(AgentSessionId::try_new("assigned-session").unwrap())
            .name(ProcessName::try_new("Ada").unwrap())
            .tool(AgentTool::Claude)
            .project(PathBuf::from("/here/muster.yml"))
            .launch_command(CommandLine::try_new("claude").unwrap())
            .state(AgentSessionState::Open)
            .build();
        let id = session.id().clone();

        let (app, _sessions, spawns) = agent_app(vec![session]);

        assert_eq!(
            app.workspace.processes()[0].agent_session_id().as_ref(),
            Some(&id)
        );
        assert_eq!(
            spawns.requests.borrow()[0]
                .command()
                .as_ref()
                .unwrap()
                .as_ref(),
            "claude --session-id assigned-session"
        );
    }

    /// An open session whose provider ID was never captured returns as a
    /// stopped row that can be closed instead of becoming stranded in history.
    #[test]
    fn startup_restores_an_uncaptured_session_as_a_closable_stopped_row() {
        let session = AgentSession::builder()
            .id(AgentSessionId::generate().unwrap())
            .name(ProcessName::try_new("Ada").unwrap())
            .tool(AgentTool::Codex)
            .project(PathBuf::from("/here/muster.yml"))
            .launch_command(CommandLine::try_new("codex").unwrap())
            .state(AgentSessionState::Open)
            .build();
        let id = session.id().clone();

        let (mut app, sessions, spawns) = agent_app(vec![session]);

        let restored = app.workspace.selected_process().unwrap();
        let pane = *restored.id();
        assert_eq!(restored.agent_session_id().as_ref(), Some(&id));
        assert_eq!(*restored.state(), ProcessState::Pending);
        assert!(!app.panes.contains_key(&pane));
        assert!(spawns.requests.borrow().is_empty());
        assert_eq!(app.notice.as_deref(), Some(AGENT_SESSION_NOT_RESUMABLE));

        press(&mut app, KeyCode::Char('d'));

        assert!(app.workspace.process(pane).is_none());
        assert_eq!(
            *sessions.sessions.borrow().last().unwrap().state(),
            AgentSessionState::Closed
        );
    }

    /// A one-key launch surfaces durable-state failures after the picker has
    /// closed instead of failing without visible feedback.
    #[test]
    fn quick_agent_launch_reports_session_store_failures() {
        let (mut app, _recorder) = flow_app(vec![], empty_workspace_config(), "/here/muster.yml");
        app.set_agent_session_store(Box::new(FailingAgentSessionStore));
        app.open_agent_picker();

        press(&mut app, KeyCode::Enter);

        assert!(
            app.workspace
                .processes()
                .iter()
                .all(|process| { *process.origin() != ProcessOrigin::Session })
        );
        assert!(
            app.notice
                .as_deref()
                .is_some_and(|notice| notice.contains(AGENT_SESSION_STORE_ERROR))
        );
    }

    /// The `a` flow defaults to the first preset and launches on submission.
    #[test]
    fn add_hotkey_launches_the_default_agent_preset() {
        let (mut app, _recorder) = flow_app(vec![], empty_workspace_config(), "/here/muster.yml");

        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.form().unwrap().form.title(), ADD_PROCESS_TITLE);
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.overlay, Some(Overlay::AgentPicker(_))));
        press(&mut app, KeyCode::Enter);

        let session = app.workspace.selected_process().unwrap();
        assert_eq!(*session.agent_tool(), Some(AgentTool::Claude));
        assert_eq!(*session.origin(), ProcessOrigin::Session);
    }

    /// Opening advanced customization from Custom preserves the custom provider
    /// instead of falling back to the first choice.
    #[test]
    fn custom_agent_form_preserves_the_selected_provider() {
        let (mut app, _recorder) = flow_app(vec![], empty_workspace_config(), "/here/muster.yml");
        app.open_agent_session_form(AgentTool::Custom);

        assert_eq!(
            app.form().unwrap().form.values()[0],
            AgentTool::Custom.to_string()
        );
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        type_text(&mut app, "my-agent");
        press(&mut app, KeyCode::Enter);

        let session = app.workspace.selected_process().unwrap();
        assert_eq!(*session.agent_tool(), Some(AgentTool::Custom));
        assert_eq!(session.command().as_ref().unwrap().as_ref(), "my-agent");
    }

    /// An incomplete placeholder-free resume template remains in the advanced
    /// form with a visible error and is never persisted or launched.
    #[test]
    fn invalid_resume_templates_are_rejected_before_agent_launch() {
        let (mut app, sessions, spawns) = agent_app(Vec::new());
        app.open_agent_session_form(AgentTool::Codex);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        type_text(&mut app, "agent --resume \"");

        press(&mut app, KeyCode::Enter);

        assert_eq!(
            app.form().unwrap().error.as_deref(),
            Some(AGENT_RESUME_TEMPLATE_INVALID)
        );
        assert!(sessions.sessions.borrow().is_empty());
        assert!(spawns.requests.borrow().is_empty());
        assert!(app.workspace.processes().is_empty());
    }

    /// A composed launch command must name its own resume behavior before the
    /// session is persisted, avoiding a later resume against the wrong command.
    #[test]
    fn compound_agent_commands_require_an_explicit_resume_template() {
        let (mut app, sessions, spawns) = agent_app(Vec::new());
        app.create_agent_session(
            AgentTool::Codex,
            None,
            Some(CommandLine::try_new("codex | tee agent.log").unwrap()),
            None,
        );

        assert_eq!(app.notice.as_deref(), Some(AGENT_COMPOUND_RESUME_REQUIRED));
        assert!(sessions.sessions.borrow().is_empty());
        assert!(spawns.requests.borrow().is_empty());
    }

    /// An unsupported fresh Claude composition must fail before durable state
    /// is written, even when its explicit resume template is valid.
    #[test]
    fn unsupported_fresh_agent_launch_is_not_persisted() {
        let (mut app, sessions, spawns) = agent_app(Vec::new());
        app.create_agent_session(
            AgentTool::Claude,
            None,
            Some(CommandLine::try_new("claude | tee agent.log").unwrap()),
            Some(CommandLine::try_new("claude --resume {session_id} | tee agent.log").unwrap()),
        );

        assert_eq!(app.notice.as_deref(), Some(AGENT_COMMAND_REQUIRED));
        assert!(sessions.sessions.borrow().is_empty());
        assert!(spawns.requests.borrow().is_empty());
    }

    /// Closing a live session waits for exit before removing its workspace row.
    #[test]
    fn closing_a_live_agent_session_retires_it_after_exit() {
        let (mut app, _recorder) = flow_app(vec![], empty_workspace_config(), "/here/muster.yml");
        app.launch_agent_session(&["Claude".to_string(), String::new(), String::new()]);
        let pane = *app.workspace.selected_process().unwrap().id();
        let generation = app.generations[&pane];

        app.handle_key(KeyEvent::new(
            KeyCode::Char(LEADER_KEY),
            KeyModifiers::CONTROL,
        ));
        press(&mut app, KeyCode::Char('d'));
        assert!(matches!(
            app.overlay,
            Some(Overlay::ConfirmSessionClose { .. })
        ));
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(app.focus, Focus::Sidebar);
        assert_eq!(
            app.panes.get(&pane).unwrap().config_membership,
            ConfigMembership::RetireOnExit
        );

        app.handle_output(pane, generation, ProcessOutput::Exited(ExitOutcome::Failed));
        assert!(app.workspace.process(pane).is_none());
        assert!(!app.panes.contains_key(&pane));
        assert_eq!(app.focus, Focus::Sidebar);
    }

    /// A failed hard-stop leaves durable history open and the live pane tracked,
    /// while a later successful retry closes and schedules that pane to retire.
    #[test]
    fn failed_agent_session_stop_does_not_persist_a_close() {
        let (mut app, sessions, _spawns) = agent_app(Vec::new());
        app.create_agent_session(AgentTool::Claude, None, None, None);
        let pane = *app.workspace.selected_process().unwrap().id();
        let signals = Arc::new(RetryStopSignals::default());
        app.panes.get_mut(&pane).unwrap().handle = Some(Box::new(RetryStopHandle {
            signals: signals.clone(),
            terminate_outcome: TerminateOutcome::Fails,
        }));

        app.close_agent_session(pane);

        assert_eq!(signals.kills.load(Ordering::SeqCst), 1);
        assert_eq!(
            *sessions.sessions.borrow().last().unwrap().state(),
            AgentSessionState::Open
        );
        assert_eq!(
            app.panes.get(&pane).unwrap().config_membership,
            ConfigMembership::Tracked
        );
        assert!(app.panes.get(&pane).unwrap().handle.is_some());
        assert_eq!(
            *app.workspace.process(pane).unwrap().state(),
            ProcessState::Running
        );
        assert_eq!(app.notice.as_deref(), Some(STOP_DELIVERY_FAILED_NOTICE));

        app.close_agent_session(pane);

        assert_eq!(signals.kills.load(Ordering::SeqCst), 2);
        assert_eq!(
            *sessions.sessions.borrow().last().unwrap().state(),
            AgentSessionState::Closed
        );
        assert_eq!(
            app.panes.get(&pane).unwrap().config_membership,
            ConfigMembership::RetireOnExit
        );
        assert_eq!(
            *app.workspace.process(pane).unwrap().state(),
            ProcessState::Stopping
        );
    }

    /// A delivered stop with failed durable persistence remains a tracked row
    /// and an open history record so its eventual exit cannot lose the session.
    #[test]
    fn failed_session_close_persistence_keeps_the_session_recoverable() {
        let (mut app, sessions, _spawns) = agent_app(Vec::new());
        app.create_agent_session(AgentTool::Claude, None, None, None);
        let pane = *app.workspace.selected_process().unwrap().id();
        app.set_agent_session_store(Box::new(FailingAgentSessionStore));

        app.close_agent_session(pane);

        assert_eq!(
            *sessions.sessions.borrow().last().unwrap().state(),
            AgentSessionState::Open
        );
        assert_eq!(
            app.panes.get(&pane).unwrap().config_membership,
            ConfigMembership::Tracked
        );
        assert_eq!(
            *app.workspace.process(pane).unwrap().state(),
            ProcessState::Stopping
        );
        assert!(
            app.notice
                .as_deref()
                .is_some_and(|notice| notice.contains(AGENT_SESSION_STORE_ERROR))
        );
    }

    /// Delayed retirement of a closing session keeps a later selected process
    /// selected even though its sidebar index shifts.
    #[test]
    fn closing_session_exit_preserves_a_later_process_selection() {
        let (mut app, _recorder) = flow_app(vec![], empty_workspace_config(), "/here/muster.yml");
        for (id, name) in [(PANE + 1, "q"), (PANE + 2, "r")] {
            app.workspace.insert_in_section(
                Process::builder()
                    .id(PaneId::new(id))
                    .name(ProcessName::try_new(name).unwrap())
                    .kind(ProcessKind::Terminal)
                    .build(),
            );
        }
        app.launch_agent_session(&["Claude".to_string(), String::new(), String::new()]);
        let session = *app.workspace.selected_process().unwrap().id();
        let generation = app.generations[&session];

        app.handle_key(KeyEvent::new(
            KeyCode::Char(LEADER_KEY),
            KeyModifiers::CONTROL,
        ));
        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Char('y'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(
            app.workspace.selected_process().unwrap().name().as_ref(),
            "q"
        );

        app.handle_output(
            session,
            generation,
            ProcessOutput::Exited(ExitOutcome::Failed),
        );

        assert_eq!(
            app.workspace.selected_process().unwrap().name().as_ref(),
            "q"
        );
        assert_eq!(app.focus, Focus::Sidebar);
    }

    /// Closing an attached stopped session detaches before the adjacent process
    /// becomes selected.
    #[test]
    fn closing_a_stopped_attached_session_returns_to_the_sidebar() {
        let (mut app, _recorder) = flow_app(vec![], empty_workspace_config(), "/here/muster.yml");
        app.launch_agent_session(&["Claude".to_string(), String::new(), String::new()]);
        let pane = *app.workspace.selected_process().unwrap().id();
        let generation = app.generations[&pane];
        app.handle_output(
            pane,
            generation,
            ProcessOutput::Exited(ExitOutcome::Succeeded),
        );

        app.handle_key(KeyEvent::new(
            KeyCode::Char(LEADER_KEY),
            KeyModifiers::CONTROL,
        ));
        press(&mut app, KeyCode::Char('d'));

        assert!(app.workspace.process(pane).is_none());
        assert_eq!(app.focus, Focus::Sidebar);
        assert_eq!(
            app.workspace.selected_process().unwrap().name().as_ref(),
            "p"
        );
    }

    /// Codex activity follows title changes instead of ordinary output.
    #[test]
    fn a_codex_session_uses_title_changes_instead_of_plain_output() {
        let (mut app, _recorder) = flow_app(vec![], empty_workspace_config(), "/here/muster.yml");
        app.launch_agent_session(&["Codex".to_string(), String::new(), String::new()]);
        let pane = *app.workspace.selected_process().unwrap().id();
        let generation = app.generations[&pane];

        app.handle_output(
            pane,
            generation,
            ProcessOutput::Chunk(b"ordinary output".to_vec()),
        );
        assert_eq!(
            *app.workspace.process(pane).unwrap().activity(),
            ActivityState::Idle
        );

        app.handle_output(
            pane,
            generation,
            ProcessOutput::Chunk(b"\x1b]2;Codex working\x07".to_vec()),
        );
        assert_eq!(
            *app.workspace.process(pane).unwrap().activity(),
            ActivityState::Working
        );
    }

    /// Live config reconciliation never treats a runtime session as a spec.
    #[test]
    fn config_reconciliation_keeps_runtime_agent_sessions() {
        let (mut app, _recorder) = flow_app(vec![], empty_workspace_config(), "/here/muster.yml");
        app.launch_agent_session(&["Gemini".to_string(), String::new(), String::new()]);
        let pane = *app.workspace.selected_process().unwrap().id();

        app.handle_config_changed(PathBuf::from("/here/muster.yml"));

        assert_eq!(
            *app.workspace.process(pane).unwrap().origin(),
            ProcessOrigin::Session
        );
    }

    /// Runtime sessions do not consume occurrences when a matching configured
    /// agent's autostart field is mapped back to YAML.
    #[test]
    fn autostart_occurrences_exclude_identical_agent_sessions() {
        let spec = ProcessSpec::builder()
            .name(ProcessName::try_new("agent").unwrap())
            .command(Some(CommandLine::try_new("true").unwrap()))
            .autostart(Some(false))
            .build();
        let config = WorkspaceConfig::builder()
            .agents(vec![spec.clone()])
            .terminals(vec![])
            .commands(vec![])
            .build();
        let session = Process::builder()
            .id(PaneId::new(PANE))
            .name(spec.name().clone())
            .kind(ProcessKind::Agent)
            .agent_tool(Some(AgentTool::Custom))
            .origin(ProcessOrigin::Session)
            .command(spec.command().clone())
            .autostart(false)
            .build();
        let configured = spec.to_process(PaneId::new(PANE + 1), ProcessKind::Agent);
        let recorder = Recorder::default();
        let (sender, _receiver) = bounded(16);
        let workspace = Workspace::builder()
            .processes(vec![session, configured])
            .selected_index(1)
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

        assert_eq!(
            recorder.workspaces.borrow()[0].1.agents()[0].autostart(),
            &Some(true)
        );
        assert!(
            *app.workspace.selected_process().unwrap().autostart(),
            "the selected configured row reflects the persisted edit"
        );
        assert!(app.notice.is_none());
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
        app.add_configured_process(ProcessKind::Terminal, &[
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

    #[cfg(unix)]
    /// Verifies an unregistered symlink alias does not make its registered
    /// target appear active in the switcher.
    #[test]
    fn switcher_keeps_a_symlink_alias_distinct_from_its_target() {
        use std::{fs, os::unix::fs::symlink};

        let dir = std::env::temp_dir().join(format!("muster-switcher-link-{}", std::process::id()));
        let workspace_dir = dir.join("workspace");
        let shared_dir = dir.join("shared");
        let target = shared_dir.join(PROJECT_CONFIG_FILE);
        let alias = workspace_dir.join(PROJECT_CONFIG_FILE);
        fs::create_dir_all(&workspace_dir).unwrap();
        fs::create_dir_all(&shared_dir).unwrap();
        fs::write(&target, "").unwrap();
        symlink(&target, &alias).unwrap();
        let projects = vec![
            Project::builder()
                .name(ProjectName::try_new("target").unwrap())
                .config(target.clone())
                .build(),
        ];
        let mut app = switcher_app(projects, empty_workspace_config(), alias.to_str().unwrap());

        app.open_switcher();

        let switcher = app.switcher().expect("switcher is open");
        assert_eq!(switcher.current, None, "the target is not marked current");
        app.handle_switcher_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            app.pending_switch
                .as_ref()
                .map(|pending| &pending.config_path),
            Some(&target),
            "enter explicitly switches from the alias to the target"
        );
        fs::remove_dir_all(dir).unwrap();
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

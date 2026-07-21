use std::{
    ops::ControlFlow,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, after, bounded, never, select, unbounded};
use crossterm::event;
use ratatui::layout::{Rect, Size};
use typed_builder::TypedBuilder;

use super::{TerminalGuard, app::App, event::RuntimeEvent, watch::NotifyConfigWatcher};
use crate::{
    application::Workspace,
    domain::port::{Notifier, PathCompleter, ProcessRunner, ProjectRegistry, SettingsStore},
    error::Result,
};

/// The driven adapters the TUI runs on, bundled so [`run`] takes one wiring
/// object instead of a long argument list. Built at the composition root and
/// consumed (moved) into the app. No `Getters`: the fields are `Box<dyn _>` and
/// are moved out once, not borrowed.
#[derive(TypedBuilder)]
pub struct Adapters {
    runner: Box<dyn ProcessRunner>,
    registry: Box<dyn ProjectRegistry>,
    completer: Box<dyn PathCompleter + Send>,
    notifier: Box<dyn Notifier>,
    settings_store: Box<dyn SettingsStore>,
}

/// Poll timeout for the input reader thread.
const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Bounded output-channel capacity; back-pressures noisy PTYs so memory stays
/// bounded instead of growing with an ever-larger backlog.
const OUTPUT_CAPACITY: usize = 1024;
/// Maximum output events drained per iteration before a single redraw.
const MAX_BATCH: usize = 512;

/// Runs the TUI. Terminal input flows on its own unbounded channel drained with
/// priority, so keystrokes are never blocked behind a flood of process output on
/// the bounded output channel.
///
/// # Errors
/// Returns an error if querying the terminal size or drawing a frame fails.
pub fn run(
    guard: &mut TerminalGuard,
    workspace: Workspace,
    adapters: Adapters,
    current_config: PathBuf,
) -> Result<()> {
    let Adapters {
        runner,
        registry,
        completer,
        notifier,
        settings_store,
    } = adapters;
    let (control_tx, control_rx) = unbounded();
    let (output_tx, output_rx) = bounded(OUTPUT_CAPACITY);
    let watch_tx = output_tx.clone();
    spawn_input_thread(control_tx);

    let area = size_to_rect(guard.terminal_mut().size()?);
    let mut app = App::new(
        workspace,
        runner,
        output_tx,
        area,
        completer,
        registry,
        current_config,
    );
    app.spawn_completion_worker();
    app.set_config_watcher(Box::new(NotifyConfigWatcher::new(watch_tx)));
    app.set_notifier(notifier);
    app.set_settings_store(settings_store);
    app.start();

    // Children are running now, so shut them down on every return path,
    // including a draw error, rather than leaking them.
    let result = run_loop(guard, &mut app, &control_rx, &output_rx);
    app.shutdown();
    result
}

/// Drives the draw/update loop until the user quits or terminal input closes.
///
/// # Errors
/// Returns an error if drawing a frame fails.
fn run_loop(
    guard: &mut TerminalGuard,
    app: &mut App,
    control_rx: &Receiver<RuntimeEvent>,
    output_rx: &Receiver<RuntimeEvent>,
) -> Result<()> {
    guard.terminal_mut().draw(|frame| app.render(frame))?;
    while app.is_running() {
        match drain(app, control_rx, output_rx) {
            ControlFlow::Break(()) => break,
            ControlFlow::Continue(redraw) if redraw && app.is_running() => {
                guard.terminal_mut().draw(|frame| app.render(frame))?;
            },
            ControlFlow::Continue(_) => {},
        }
    }
    Ok(())
}

/// Blocks for the next event or activity deadline, then drains all pending input
/// (priority) followed by a bounded batch of output. Returns whether to redraw,
/// or `Break` when the loop should stop.
fn drain(
    app: &mut App,
    control_rx: &Receiver<RuntimeEvent>,
    output_rx: &Receiver<RuntimeEvent>,
) -> ControlFlow<(), bool> {
    let activity_timeout = app
        .next_activity_deadline()
        .map(|deadline| after(deadline.saturating_duration_since(Instant::now())))
        .unwrap_or_else(never);
    let mut redraw = false;
    select! {
        recv(control_rx) -> msg => match msg {
            Ok(event) => if !apply(app, event) {
                return ControlFlow::Break(());
            } else {
                redraw = true;
            },
            Err(_) => return ControlFlow::Break(()),
        },
        recv(output_rx) -> msg => if let Ok(event) = msg {
            if !apply(app, event) {
                return ControlFlow::Break(());
            }
            redraw = true;
        },
        recv(activity_timeout) -> now => if let Ok(now) = now {
            redraw = app.expire_quiet_activity(now);
        },
    }
    while let Ok(event) = control_rx.try_recv() {
        if !apply(app, event) {
            return ControlFlow::Break(());
        }
        redraw = true;
    }
    for _ in 0..MAX_BATCH {
        match output_rx.try_recv() {
            Ok(event) => {
                if !apply(app, event) {
                    return ControlFlow::Break(());
                }
                redraw = true;
            },
            Err(_) => break,
        }
    }
    ControlFlow::Continue(redraw)
}

/// Applies one event to the app; returns `false` when the loop should stop.
fn apply(app: &mut App, event: RuntimeEvent) -> bool {
    match event {
        RuntimeEvent::Input(event) => app.handle_input(event),
        RuntimeEvent::Output {
            pane,
            generation,
            output,
        } => app.handle_output(pane, generation, output),
        RuntimeEvent::Respawn { pane, generation } => app.handle_respawn(pane, generation),
        RuntimeEvent::ForceStop { pane, generation } => app.handle_force_stop(pane, generation),
        RuntimeEvent::Completions {
            generation,
            candidates,
        } => app.handle_completions(generation, candidates),
        RuntimeEvent::ConfigChanged { path } => app.handle_config_changed(path),
        RuntimeEvent::InputClosed => return false,
    }
    true
}

/// Spawns a thread forwarding crossterm input onto the control channel, sending
/// `InputClosed` if the input source errors so the loop never blocks forever.
fn spawn_input_thread(sender: Sender<RuntimeEvent>) {
    thread::spawn(move || {
        loop {
            match event::poll(INPUT_POLL_INTERVAL) {
                Ok(true) => match event::read() {
                    Ok(event) => {
                        if sender.send(RuntimeEvent::Input(event)).is_err() {
                            break;
                        }
                    },
                    Err(_) => {
                        let _ = sender.send(RuntimeEvent::InputClosed);
                        break;
                    },
                },
                Ok(false) => {},
                Err(_) => {
                    let _ = sender.send(RuntimeEvent::InputClosed);
                    break;
                },
            }
        }
    });
}

/// Converts a terminal size into a full-screen rectangle.
fn size_to_rect(size: Size) -> Rect {
    Rect::new(0, 0, size.width, size.height)
}

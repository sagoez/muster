use std::{
    iter,
    ops::ControlFlow,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, after, bounded, never, select, unbounded};
use crossterm::event;
use ratatui::{
    layout::{Rect, Size},
    style::Style,
};
use typed_builder::TypedBuilder;

use super::{TerminalGuard, app::App, event::RuntimeEvent, watch::NotifyConfigWatcher};
use crate::{
    application::Workspace,
    domain::port::{
        AgentSessionStore, EditorLauncher, Notifier, PathCompleter, ProcessMetrics, ProcessRunner,
        ProjectRegistry, SettingsStore,
    },
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
    agent_session_store: Box<dyn AgentSessionStore>,
    editor_launcher: Box<dyn EditorLauncher>,
    process_metrics: Box<dyn ProcessMetrics>,
}

/// Poll timeout for the input reader thread.
const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Bounded output-channel capacity; back-pressures noisy PTYs so memory stays
/// bounded instead of growing with an ever-larger backlog.
const OUTPUT_CAPACITY: usize = 1024;
/// Maximum output events drained per iteration before a single redraw.
const MAX_BATCH: usize = 512;

/// Runs the TUI. Terminal input flows on its own unbounded channel and is drained
/// in full every iteration, so keystrokes are never dropped behind a flood of
/// process output on the bounded output channel; a bounded output batch is
/// applied first each iteration so a key is encoded against any keyboard-mode
/// change the child just negotiated.
///
/// # Errors
/// Returns an error if querying the terminal size or drawing a frame fails.
pub fn run(
    guard: &mut TerminalGuard,
    workspace: Workspace,
    adapters: Adapters,
    current_config: PathBuf,
    selection_style: Style,
) -> Result<()> {
    let Adapters {
        runner,
        registry,
        completer,
        notifier,
        settings_store,
        agent_session_store,
        editor_launcher,
        process_metrics,
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
    app.set_agent_session_store(agent_session_store);
    app.set_process_metrics(process_metrics);
    app.set_selection_style(selection_style);
    app.start();

    // Children are running now, so shut them down on every return path,
    // including a draw error, rather than leaking them.
    let result = run_loop(
        guard,
        &mut app,
        &control_rx,
        &output_rx,
        editor_launcher.as_ref(),
    );
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
    editor_launcher: &dyn EditorLauncher,
) -> Result<()> {
    guard.terminal_mut().draw(|frame| app.render(frame))?;
    while app.is_running() {
        let outcome = drain(app, control_rx, output_rx);
        if let Some(text) = app.take_pending_clipboard() {
            guard.copy_to_clipboard(&text)?;
        }
        if let Some(shape) = app.take_pending_pointer_shape() {
            guard.set_pointer_shape(shape)?;
        }
        // Skip a queued open-editor request when quitting: no point opening a
        // window on the way out.
        if !shutting_down(&outcome, app.is_running())
            && let Some(directory) = app.take_pending_editor()
        {
            // The editor opens in its own detached window and returns at once, so
            // muster keeps running with no terminal handoff to manage. Redraw to
            // surface the resulting toast or error notice.
            app.report_editor_result(editor_launcher.open(&directory));
            guard.terminal_mut().draw(|frame| app.render(frame))?;
        }
        match outcome {
            ControlFlow::Break(()) => break,
            ControlFlow::Continue(redraw) if redraw && app.is_running() => {
                app.refresh_selection_view();
                guard.terminal_mut().draw(|frame| app.render(frame))?;
            },
            ControlFlow::Continue(_) => {},
        }
    }
    Ok(())
}

/// Blocks for the next event or activity deadline, then drains queued output ahead
/// of the input already pending (see [`drain_pending`]), so a key is encoded against
/// keyboard-mode changes the child emitted in the same wake-up. Returns whether to
/// redraw, or `Break` when the loop should stop.
fn drain(
    app: &mut App,
    control_rx: &Receiver<RuntimeEvent>,
    output_rx: &Receiver<RuntimeEvent>,
) -> ControlFlow<(), bool> {
    let activity_timeout = app
        .next_activity_deadline()
        .map(|deadline| after(deadline.saturating_duration_since(Instant::now())))
        .unwrap_or_else(never);
    let activity_frame_timeout = app
        .next_activity_frame_deadline()
        .map(|deadline| after(deadline.saturating_duration_since(Instant::now())))
        .unwrap_or_else(never);
    let selection_timeout = app
        .next_selection_deadline()
        .map(|deadline| after(deadline.saturating_duration_since(Instant::now())))
        .unwrap_or_else(never);
    let toast_timeout = app
        .next_toast_deadline()
        .map(|deadline| after(deadline.saturating_duration_since(Instant::now())))
        .unwrap_or_else(never);
    let notice_timeout = app
        .next_notice_deadline()
        .map(|deadline| after(deadline.saturating_duration_since(Instant::now())))
        .unwrap_or_else(never);
    let metrics_timeout = app
        .next_metrics_deadline()
        .map(|deadline| after(deadline.saturating_duration_since(Instant::now())))
        .unwrap_or_else(never);
    let mut redraw = false;
    // A key consumed by `select!` is held back rather than applied inline, so any
    // process output already queued (which may carry a keyboard-mode change) is
    // observed before the key is encoded against the negotiated protocol.
    let mut buffered_input = None;
    select! {
        recv(control_rx) -> msg => match msg {
            Ok(event) => buffered_input = Some(event),
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
        recv(activity_frame_timeout) -> now => if let Ok(now) = now {
            redraw = app.advance_activity_frame(now);
        },
        recv(selection_timeout) -> now => if let Ok(now) = now {
            redraw = app.advance_selection(now);
        },
        recv(toast_timeout) -> now => if let Ok(now) = now {
            redraw = app.expire_toast(now);
        },
        recv(notice_timeout) -> now => if let Ok(now) = now {
            redraw = app.expire_notice(now);
        },
        recv(metrics_timeout) -> now => if let Ok(now) = now {
            redraw = app.sample_metrics(now);
        },
    }
    match drain_pending(control_rx, output_rx, buffered_input, redraw, |event| {
        apply(app, event)
    }) {
        ControlFlow::Break(()) => return ControlFlow::Break(()),
        ControlFlow::Continue(updated) => redraw = updated,
    }
    let now = Instant::now();
    redraw |= app.advance_activity_frame(now);
    redraw |= app.advance_selection(now);
    redraw |= app.expire_toast(now);
    redraw |= app.expire_notice(now);
    redraw |= app.sample_metrics(now);
    ControlFlow::Continue(redraw)
}

/// Applies the events pending after a `select!` wake-up, output before input, so a
/// key is encoded against the negotiation state the child had already emitted.
///
/// Both queues are snapshotted up front. Output leads: with a key waiting the whole
/// output snapshot is drained (even past `MAX_BATCH`) so no queued mode change is
/// missed; otherwise the batch is capped so a flood cannot delay a redraw.
/// Snapshotting - rather than looping until empty - keeps a continuously refilling
/// child from spinning the drain forever and starving the key, quit, timers, and
/// redraws. Only the input snapshot is applied, so a key that lands mid-drain is
/// deferred to the next iteration, where output again leads it. `apply_event`
/// returns `false` to stop the loop; the returned flag reports whether a redraw is
/// warranted.
fn drain_pending(
    control_rx: &Receiver<RuntimeEvent>,
    output_rx: &Receiver<RuntimeEvent>,
    buffered_input: Option<RuntimeEvent>,
    mut redraw: bool,
    mut apply_event: impl FnMut(RuntimeEvent) -> bool,
) -> ControlFlow<(), bool> {
    // Sample each backlog once. `input_pending` is derived from the same
    // `input_backlog` count that bounds the input drain, not a separate
    // `is_empty()` call: a key arriving between the two would otherwise leave
    // `input_pending` false while `input_backlog` counted it, capping the output
    // drain and applying the key ahead of queued negotiation.
    let output_backlog = output_rx.len();
    let input_backlog = control_rx.len();
    let input_pending = buffered_input.is_some() || input_backlog > 0;
    let output_limit = if input_pending {
        output_backlog
    } else {
        output_backlog.min(MAX_BATCH)
    };
    for _ in 0..output_limit {
        match output_rx.try_recv() {
            Ok(event) => {
                if !apply_event(event) {
                    return ControlFlow::Break(());
                }
                redraw = true;
            },
            Err(_) => break,
        }
    }
    let input = buffered_input
        .into_iter()
        .chain(iter::from_fn(|| control_rx.try_recv().ok()).take(input_backlog));
    for event in input {
        if !apply_event(event) {
            return ControlFlow::Break(());
        }
        redraw = true;
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
        RuntimeEvent::ForceStop {
            pane,
            spawn_generation,
            shutdown_generation,
        } => app.handle_force_stop(pane, spawn_generation, shutdown_generation),
        RuntimeEvent::Completions {
            generation,
            candidates,
        } => app.handle_completions(generation, candidates),
        RuntimeEvent::ConfigChanged { path } => app.handle_config_changed(path),
        RuntimeEvent::InputClosed => return false,
    }
    true
}

/// Whether the loop should exit this iteration instead of running a deferred
/// action such as an editor launch. A quit key drained in the same batch clears
/// `is_running`, while closed or errored input yields a `Break` outcome; either
/// means a pending editor must not open on the way out.
fn shutting_down(outcome: &ControlFlow<(), bool>, is_running: bool) -> bool {
    matches!(outcome, ControlFlow::Break(())) || !is_running
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

#[cfg(test)]
mod tests {
    use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::{
        adapter::tui::spawn_generation::SpawnGeneration,
        domain::{pty::ProcessOutput, value::PaneId},
    };

    /// A process-output event; its payload is irrelevant to ordering.
    fn output_event() -> RuntimeEvent {
        RuntimeEvent::Output {
            pane: PaneId::new(0),
            generation: SpawnGeneration::initial(),
            output: ProcessOutput::Chunk(Vec::new()),
        }
    }

    /// A key-input event.
    fn input_event() -> RuntimeEvent {
        RuntimeEvent::Input(CrosstermEvent::Key(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
        )))
    }

    /// A quit or closed input queued alongside an open-editor request defers to
    /// shutdown, so the editor never opens on the way out; while still running,
    /// the editor is allowed to open.
    #[test]
    fn a_queued_shutdown_preempts_the_editor() {
        assert!(
            shutting_down(&ControlFlow::Continue(true), false),
            "a quit key drained in the same batch clears is_running"
        );
        assert!(
            shutting_down(&ControlFlow::Break(()), true),
            "closed or errored input breaks the loop"
        );
        assert!(
            !shutting_down(&ControlFlow::Continue(true), true),
            "still running, so a pending editor may open"
        );
    }

    /// Output pending in the same wake-up as input is applied first, so a key is
    /// never encoded before a keyboard-mode change the child already emitted. The
    /// buffered key (the one `select!` consumed) and the queued key both follow.
    #[test]
    fn queued_output_is_applied_before_input() {
        let (control_tx, control_rx) = unbounded();
        let (output_tx, output_rx) = unbounded();
        output_tx.send(output_event()).unwrap();
        control_tx.send(input_event()).unwrap();

        let mut is_output = Vec::new();
        let outcome = drain_pending(
            &control_rx,
            &output_rx,
            Some(input_event()),
            false,
            |event| {
                is_output.push(matches!(event, RuntimeEvent::Output { .. }));
                true
            },
        );

        assert!(matches!(outcome, ControlFlow::Continue(true)));
        assert_eq!(is_output, vec![true, false, false]);
    }

    /// With a key waiting, output past `MAX_BATCH` is still drained before the
    /// key, so a mode change beyond the batch cap is not skipped.
    #[test]
    fn all_queued_output_drains_before_a_waiting_key() {
        let (_control_tx, control_rx) = unbounded::<RuntimeEvent>();
        let (output_tx, output_rx) = unbounded();
        let overflow = MAX_BATCH + 10;
        for _ in 0..overflow {
            output_tx.send(output_event()).unwrap();
        }

        let mut is_output = Vec::new();
        let outcome = drain_pending(
            &control_rx,
            &output_rx,
            Some(input_event()),
            false,
            |event| {
                is_output.push(matches!(event, RuntimeEvent::Output { .. }));
                true
            },
        );

        assert!(matches!(outcome, ControlFlow::Continue(true)));
        assert_eq!(is_output.len(), overflow + 1);
        assert!(is_output[..overflow].iter().all(|&is_output| is_output));
        assert!(!is_output[overflow]);
    }

    /// The output drain is bounded by the backlog snapshotted at the start, so a
    /// child that refills the channel while the batch runs cannot extend it
    /// indefinitely and starve the waiting key.
    #[test]
    fn output_drain_is_bounded_by_the_backlog_snapshot() {
        let (control_tx, control_rx) = unbounded::<RuntimeEvent>();
        let (output_tx, output_rx) = unbounded();
        output_tx.send(output_event()).unwrap();
        output_tx.send(output_event()).unwrap();
        control_tx.send(input_event()).unwrap();

        let refill = output_tx.clone();
        let mut outputs = 0;
        let mut refilled = false;
        let outcome = drain_pending(&control_rx, &output_rx, None, false, |event| {
            if matches!(event, RuntimeEvent::Output { .. }) {
                outputs += 1;
                if !refilled {
                    refill.send(output_event()).unwrap();
                    refilled = true;
                }
            }
            true
        });

        assert!(matches!(outcome, ControlFlow::Continue(true)));
        // Only the two events queued at entry are drained; the mid-drain refill
        // waits for the next iteration.
        assert_eq!(outputs, 2);
    }

    /// A key that arrives while the output batch is draining is left for the next
    /// iteration rather than applied against output still queued behind it.
    #[test]
    fn input_arriving_mid_batch_is_deferred() {
        let (control_tx, control_rx) = unbounded::<RuntimeEvent>();
        let (output_tx, output_rx) = unbounded();
        output_tx.send(output_event()).unwrap();
        output_tx.send(output_event()).unwrap();

        let late = control_tx.clone();
        let mut applied_input = false;
        let mut sent = false;
        let outcome = drain_pending(&control_rx, &output_rx, None, false, |event| {
            match event {
                RuntimeEvent::Output { .. } => {
                    if !sent {
                        late.send(input_event()).unwrap();
                        sent = true;
                    }
                },
                RuntimeEvent::Input(_) => applied_input = true,
                _ => {},
            }
            true
        });

        assert!(matches!(outcome, ControlFlow::Continue(true)));
        assert!(!applied_input);
        assert!(!control_rx.is_empty());
    }

    /// A key already queued on the control channel (not just the `select!`-buffered
    /// one) forces a full output drain past `MAX_BATCH`, since `input_pending` is
    /// derived from the sampled backlog.
    #[test]
    fn queued_control_input_forces_a_full_output_drain() {
        let (control_tx, control_rx) = unbounded::<RuntimeEvent>();
        let (output_tx, output_rx) = unbounded();
        let overflow = MAX_BATCH + 10;
        for _ in 0..overflow {
            output_tx.send(output_event()).unwrap();
        }
        control_tx.send(input_event()).unwrap();

        let mut outputs = 0;
        let outcome = drain_pending(&control_rx, &output_rx, None, false, |event| {
            if matches!(event, RuntimeEvent::Output { .. }) {
                outputs += 1;
            }
            true
        });

        assert!(matches!(outcome, ControlFlow::Continue(true)));
        assert_eq!(outputs, overflow);
    }

    /// With no key waiting, the output batch stays capped at `MAX_BATCH` so a
    /// flood cannot delay the next redraw; the remainder waits for the next drain.
    #[test]
    fn output_stays_capped_when_no_key_waits() {
        let (_control_tx, control_rx) = unbounded::<RuntimeEvent>();
        let (output_tx, output_rx) = unbounded();
        for _ in 0..(MAX_BATCH + 10) {
            output_tx.send(output_event()).unwrap();
        }

        let mut applied = 0;
        let outcome = drain_pending(&control_rx, &output_rx, None, false, |_| {
            applied += 1;
            true
        });

        assert!(matches!(outcome, ControlFlow::Continue(true)));
        assert_eq!(applied, MAX_BATCH);
    }

    /// An event whose application signals stop ends the drain and reports a break.
    #[test]
    fn a_stopping_event_breaks_the_drain() {
        let (_control_tx, control_rx) = unbounded();
        let (output_tx, output_rx) = unbounded();
        output_tx.send(output_event()).unwrap();

        let outcome = drain_pending(&control_rx, &output_rx, None, false, |_| false);

        assert!(matches!(outcome, ControlFlow::Break(())));
    }
}

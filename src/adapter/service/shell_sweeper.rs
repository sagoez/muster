use std::{sync::Arc, thread, time::Duration};

use crossbeam_channel::{Sender, bounded};

use super::event::ServiceEvent;
use crate::domain::{occupancy::ShellId, port::ShellRegistry};

/// How often the service checks that the shells it knows still run.
pub const SHELL_SWEEP_INTERVAL: Duration = Duration::from_secs(5);

/// Finds shells that ended, on its own thread: each one found is forgotten in
/// the registry and posted back as [`ServiceEvent::ShellsGone`]. A sweep asked
/// for while one runs is dropped. The thread ends once the sweeper is
/// dropped.
pub struct ShellSweeper {
    /// Where the loop sends the shells to check.
    requests: Sender<Vec<ShellId>>,
}

impl ShellSweeper {
    /// Starts sweeping through `shells`, posting what ended to `events`.
    pub fn spawn(shells: Arc<dyn ShellRegistry>, events: Sender<ServiceEvent>) -> Self {
        let (requests, received) = bounded::<Vec<ShellId>>(1);
        thread::spawn(move || {
            for known in received {
                let gone: Vec<ShellId> = known
                    .into_iter()
                    .filter(|shell| !shells.is_live(shell))
                    .collect();
                if gone.is_empty() {
                    continue;
                }
                for shell in &gone {
                    shells.forget(shell);
                }
                if events
                    .send(ServiceEvent::ShellsGone { shells: gone })
                    .is_err()
                {
                    return;
                }
            }
        });
        Self { requests }
    }

    /// Checks `known` off the loop, unless a sweep is in flight.
    pub fn sweep(&self, known: Vec<ShellId>) {
        let _ = self.requests.try_send(known);
    }
}

#[cfg(test)]
mod tests {
    use super::{super::fake_shells::FakeShells, *};
    use crate::domain::value::{Pid, StartToken};

    /// Longest a test waits for a sweep.
    const SWEEP_WAIT: Duration = Duration::from_secs(5);

    /// The shell running as `pid`.
    fn shell(pid: u32) -> ShellId {
        ShellId::builder()
            .pid(Pid::try_new(pid).unwrap())
            .start(StartToken::try_new(1).unwrap())
            .build()
    }

    #[test]
    fn only_ended_shells_are_forgotten_and_posted() {
        let shells = FakeShells::default();
        shells.open(shell(101));
        let (sender, events) = bounded(4);
        let sweeper = ShellSweeper::spawn(Arc::new(shells.clone()), sender);

        sweeper.sweep(vec![shell(101), shell(102)]);

        let ServiceEvent::ShellsGone { shells: gone } = events.recv_timeout(SWEEP_WAIT).unwrap()
        else {
            panic!("expected the ended shells");
        };
        assert_eq!(gone, [shell(102)]);
        assert_eq!(shells.forgotten(), [shell(102)]);

        sweeper.sweep(vec![shell(101)]);
        assert!(
            events.recv_timeout(SWEEP_WAIT / 5).is_err(),
            "nothing ended, nothing posted"
        );
    }
}

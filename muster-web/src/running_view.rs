use std::{
    ffi::c_int,
    thread::{self, JoinHandle},
    time::Instant,
};

use signal_hook::iterator::Signals;

use super::{browser::open_browser, http::Gate, live::HubHandle, web_error::WebError};

/// The name of the thread that watches the hub end.
const HUB_WATCH_THREAD: &str = "view-hub-watch";

/// `muster view`, serving: the gate behind which it answers, the floor its
/// address opens on, and the hub that owns that floor, until an exit signal
/// or the hub's end stops it.
pub struct RunningView {
    /// What admits a request.
    gate: Gate,
    /// The fragment of every address it hands out, naming the floor the
    /// page opens on; empty for none.
    fragment: String,
    /// The signals that end the view.
    exit_signals: Signals,
    /// How the hub is told to stop.
    hub: HubHandle,
    /// The hub's thread.
    hub_thread: JoinHandle<()>,
}

impl RunningView {
    /// The view serving behind `gate`, opening on `fragment`, ended by
    /// `exit_signals` or the end of the hub that `hub` reaches and
    /// `hub_thread` runs.
    pub(super) fn new(
        gate: Gate,
        fragment: String,
        exit_signals: Signals,
        hub: HubHandle,
        hub_thread: JoinHandle<()>,
    ) -> Self {
        Self {
            gate,
            fragment,
            exit_signals,
            hub,
            hub_thread,
        }
    }

    /// The address that opens the view, carrying the token in its path.
    pub fn url(&self) -> String {
        format!("{}{}", self.gate.url(), self.fragment)
    }

    /// Opens the view in the default browser at a one-time address, so the
    /// token never reaches a process's arguments. The code is minted now,
    /// with the server already serving, so its lifetime is all the
    /// browser's.
    ///
    /// # Errors
    /// Returns [`WebError::Code`] when no code can be made, or
    /// [`WebError::Browser`] when no browser could be started.
    pub fn open_in_browser(&self) -> Result<(), WebError> {
        let url = self.gate.open_url(Instant::now()).map_err(WebError::Code)?;
        open_browser(&format!("{url}{}", self.fragment)).map_err(WebError::Browser)
    }

    /// Serves until an exit signal arrives, and returns that signal's
    /// number; the signal stops the hub, which ends every stream and, through
    /// it, every poller.
    ///
    /// # Errors
    /// Returns [`WebError::HubEnded`] when the hub ended on its own first,
    /// which leaves nothing to serve.
    pub fn wait(self) -> Result<c_int, WebError> {
        let Self {
            mut exit_signals,
            hub,
            hub_thread,
            ..
        } = self;
        wait_for_end(&mut exit_signals, hub_thread, || hub.stop()).ok_or(WebError::HubEnded)
    }
}

/// The first of `signals` to arrive, or none once `hub_thread` ends first.
/// Either way it then calls `stop`, which tells the hub to end, and returns
/// once the hub's thread has.
fn wait_for_end(
    signals: &mut Signals,
    hub_thread: JoinHandle<()>,
    stop: impl FnOnce(),
) -> Option<c_int> {
    let closer = signals.handle();
    // A watch that cannot start drops the hub's thread unjoined, and only a
    // signal then ends the wait, as before the watch existed.
    let watch = thread::Builder::new()
        .name(HUB_WATCH_THREAD.to_string())
        .spawn(move || {
            let _ = hub_thread.join();
            closer.close();
        });
    let signal = signals.forever().next();
    stop();
    if let Ok(watch) = watch {
        let _ = watch.join();
    }
    signal
}

#[cfg(test)]
mod tests {
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };

    use signal_hook::{
        consts::{SIGUSR1, SIGUSR2},
        low_level::raise,
    };

    use super::*;

    /// Longest a test waits for the wait to end.
    const BOUND: Duration = Duration::from_secs(10);

    /// A hub that ends on its own ends the wait, with no signal. It waits
    /// on a signal no test raises, so the other test's cannot reach it.
    #[test]
    fn a_hub_that_ends_ends_the_wait() {
        let mut signals = Signals::new([SIGUSR1]).unwrap();
        let hub_thread = thread::spawn(|| thread::sleep(Duration::from_millis(50)));
        let started = Instant::now();

        assert_eq!(wait_for_end(&mut signals, hub_thread, || {}), None);
        assert!(started.elapsed() < BOUND);
    }

    /// A signal ends the wait with its number, after the hub is told to
    /// stop and has ended.
    #[test]
    fn a_signal_stops_the_hub_and_ends_the_wait() {
        let mut signals = Signals::new([SIGUSR2]).unwrap();
        let (stop, stopped) = mpsc::channel::<()>();
        let hub_thread = thread::spawn(move || {
            let _ = stopped.recv_timeout(BOUND);
        });
        raise(SIGUSR2).unwrap();

        assert_eq!(
            wait_for_end(&mut signals, hub_thread, move || drop(stop)),
            Some(SIGUSR2)
        );
    }
}

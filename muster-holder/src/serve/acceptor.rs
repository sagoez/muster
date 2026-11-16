use std::{
    io::{self, Write},
    net::Shutdown,
    os::unix::net::{UnixListener, UnixStream},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crossbeam_channel::{SendTimeoutError, Sender};
use muster_protocol::holder::ClientMessage;
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    io::Errno,
};

use crate::{clients::client_id::ClientId, serve::holder_event::HolderEvent};

/// Pause after an accept fails for another reason than an empty queue, such
/// as running out of file descriptors, so a listener that stays readable is
/// not spun on.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
/// The byte that wakes an acceptor to stop.
const WAKE: u8 = 0;
/// A poll that returns at once.
const NO_WAIT: Timespec = Timespec {
    tv_sec: 0,
    tv_nsec: 0,
};

/// One thread accepting connections on the holder's listener and handing them
/// to the holder loop. Dropping it wakes the thread at once and joins it, so
/// its listener is closed by the time the drop returns: a socket bound again
/// leaves no acceptor or listening descriptor behind.
pub(super) struct Acceptor {
    /// The end that wakes the thread to stop.
    wake: UnixStream,
    /// The accepting thread, until joined.
    thread: Option<JoinHandle<()>>,
}

impl Acceptor {
    /// Accepts connections on `listener` on a thread of its own, numbering
    /// each from `numbers`, which every acceptor of the holder shares, so a
    /// connection to a socket bound again never takes the number of one
    /// still open, and counting itself in `running` while it runs.
    ///
    /// # Errors
    /// Returns the I/O error from making the wake pair or making the listener
    /// non-blocking.
    pub(super) fn start(
        listener: UnixListener,
        events: Sender<HolderEvent>,
        numbers: Arc<AtomicU64>,
        running: Arc<AtomicUsize>,
    ) -> io::Result<Self> {
        let (wake, woken) = UnixStream::pair()?;
        listener.set_nonblocking(true)?;
        running.fetch_add(1, Ordering::SeqCst);
        let thread = thread::spawn(move || {
            accept_until_woken(&listener, &woken, &events, &numbers);
            running.fetch_sub(1, Ordering::SeqCst);
        });
        Ok(Self {
            wake,
            thread: Some(thread),
        })
    }
}

impl Drop for Acceptor {
    /// Wakes the thread and waits for it, closing its listener.
    fn drop(&mut self) {
        let _ = self.wake.write_all(&[WAKE]);
        let _ = self.wake.shutdown(Shutdown::Write);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Accepts connections on `listener` until `woken` is readable or the holder
/// loop is gone, giving each a reader thread that forwards its messages to
/// the loop.
fn accept_until_woken(
    listener: &UnixListener,
    woken: &UnixStream,
    events: &Sender<HolderEvent>,
    numbers: &AtomicU64,
) {
    loop {
        let mut ready = [
            PollFd::new(listener, PollFlags::IN),
            PollFd::new(woken, PollFlags::IN),
        ];
        match poll(&mut ready, None) {
            Ok(_) => {},
            Err(Errno::INTR) => continue,
            Err(_) => return,
        }
        if !ready[1].revents().is_empty() {
            return;
        }
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            },
            Err(_) => {
                thread::sleep(ACCEPT_BACKOFF);
                continue;
            },
        };
        let Some(reader) = stream
            .set_nonblocking(false)
            .and_then(|()| stream.try_clone())
            .ok()
        else {
            continue;
        };
        let client = ClientId::new(numbers.fetch_add(1, Ordering::SeqCst));
        if !deliver(events, HolderEvent::Connected { client, stream }, woken) {
            return;
        }
        spawn_reader(client, reader, events.clone());
    }
}

/// Hands `event` to the holder loop, waiting while its inbox is full, and
/// gives up once `woken` asks the thread to stop, since the loop that asked
/// is the one that would drain the inbox and waits for this thread. Returns
/// whether the event was handed over.
fn deliver(events: &Sender<HolderEvent>, mut event: HolderEvent, woken: &UnixStream) -> bool {
    loop {
        match events.send_timeout(event, ACCEPT_BACKOFF) {
            Ok(()) => return true,
            Err(SendTimeoutError::Timeout(unsent)) if !is_woken(woken) => event = unsent,
            Err(_) => return false,
        }
    }
}

/// Whether `woken` has been written to or closed, asking the thread to stop.
fn is_woken(woken: &UnixStream) -> bool {
    let mut ready = [PollFd::new(woken, PollFlags::IN)];
    poll(&mut ready, Some(&NO_WAIT)).is_ok_and(|_| !ready[0].revents().is_empty())
}

/// Forwards one client's messages to the holder loop until the connection
/// closes or sends something unreadable, then reports the disconnect.
fn spawn_reader(client: ClientId, mut stream: UnixStream, events: Sender<HolderEvent>) {
    thread::spawn(move || {
        while let Ok(message) = ClientMessage::read_from(&mut stream) {
            if events
                .send(HolderEvent::Message { client, message })
                .is_err()
            {
                return;
            }
        }
        let _ = events.send(HolderEvent::Disconnected { client });
    });
}

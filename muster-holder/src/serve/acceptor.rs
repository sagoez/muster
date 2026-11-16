use std::{
    io::{self, Write},
    net::Shutdown,
    os::unix::net::{UnixListener, UnixStream},
    sync::Arc,
    thread,
    time::Duration,
};

use crossbeam_channel::{SendTimeoutError, Sender};
use muster_protocol::holder::ClientMessage;
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    io::Errno,
};

use crate::{
    clients::client_id::ClientId,
    serve::{
        acceptor_counts::AcceptorCounts, holder_event::HolderEvent, open_listener::OpenListener,
    },
};

/// Pause after a poll or an accept fails for a reason that passes, such as
/// running out of memory or file descriptors, so a listener that stays
/// readable is not spun on.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
/// What a poll reports of a listener that can accept nothing more, as one
/// shut down: it would read as ready forever while every accept finds
/// nothing.
const LISTENER_FAILED: PollFlags = PollFlags::HUP.union(PollFlags::ERR).union(PollFlags::NVAL);
/// Why an acceptor whose listener hung up ended.
const LISTENER_HUNG_UP: &str = "the holder socket's listener hung up";
/// Why an acceptor a test refused did not start.
#[cfg(test)]
const REFUSED_START: &str = "a test refused the acceptor's start";
/// The byte that wakes an acceptor to stop.
const WAKE: u8 = 0;
/// A poll that returns at once.
const NO_WAIT: Timespec = Timespec {
    tv_sec: 0,
    tv_nsec: 0,
};

/// One thread accepting connections on the holder's listener and handing them
/// to the holder loop. Dropping it wakes the thread to stop without waiting
/// for it, so the loop never blocks on a join; the thread closes its end of
/// the listener as it returns, which takes a moment. A thread that ends on
/// its own, its listener failing, says so to the loop and is seen as ended.
pub(super) struct Acceptor {
    /// The end that wakes the thread to stop.
    wake: UnixStream,
    /// The listener, shared with the thread until it returns, and held so the
    /// socket's file stays the one bound until the acceptor is let go, even
    /// when the thread ended on its own.
    listener: Arc<OpenListener>,
}

impl Acceptor {
    /// Accepts connections on `listener` on a thread of its own, numbering
    /// each and counting itself and the listener in `counts`, which every
    /// acceptor of the holder shares.
    ///
    /// # Errors
    /// Returns the I/O error from making the wake pair, making the listener
    /// non-blocking or starting the thread.
    pub(super) fn start(
        listener: UnixListener,
        events: Sender<HolderEvent>,
        counts: &Arc<AcceptorCounts>,
    ) -> io::Result<Self> {
        #[cfg(test)]
        if counts.refuses_starts() {
            return Err(io::Error::other(REFUSED_START));
        }
        let (wake, woken) = UnixStream::pair()?;
        listener.set_nonblocking(true)?;
        let listener = Arc::new(OpenListener::new(listener, Arc::clone(counts)));
        let accepting = Arc::clone(&listener);
        let counted = Arc::clone(counts);
        counted.thread_started();
        let spawned = thread::Builder::new().spawn(move || {
            let failed =
                accept_until_woken(accepting.listener(), &woken, &events, &counted).is_err();
            drop(accepting);
            if failed {
                let _ = deliver(&events, HolderEvent::AcceptorEnded, &woken);
            }
            counted.thread_returned();
        });
        if let Err(error) = spawned {
            counts.thread_returned();
            return Err(error);
        }
        Ok(Self { wake, listener })
    }

    /// Whether the thread has returned, letting go of its listener. Until
    /// the acceptor is dropped nothing asks it to, so a thread that returned
    /// ended on its own and nothing accepts on the socket any more.
    pub(super) fn has_ended(&self) -> bool {
        Arc::strong_count(&self.listener) == 1
    }

    /// The listener the acceptor serves.
    #[cfg(test)]
    pub(super) fn listener(&self) -> &UnixListener {
        self.listener.listener()
    }
}

impl Drop for Acceptor {
    /// Wakes the thread to stop, never waiting for it.
    fn drop(&mut self) {
        let _ = self.wake.write_all(&[WAKE]);
        let _ = self.wake.shutdown(Shutdown::Write);
    }
}

/// Accepts connections on `listener` until `woken` is readable or the holder
/// loop is gone, giving each a reader thread that forwards its messages to
/// the loop. A poll or accept failure that passes is waited out and tried
/// again.
///
/// # Errors
/// Returns the error the listener failed with for good, after which nothing
/// accepts on it.
fn accept_until_woken(
    listener: &UnixListener,
    woken: &UnixStream,
    events: &Sender<HolderEvent>,
    counts: &AcceptorCounts,
) -> io::Result<()> {
    loop {
        let mut ready = [
            PollFd::new(listener, PollFlags::IN),
            PollFd::new(woken, PollFlags::IN),
        ];
        match poll(&mut ready, None) {
            Ok(_) => {},
            Err(Errno::INTR) => continue,
            Err(errno) if passes(errno) => {
                thread::sleep(ACCEPT_BACKOFF);
                continue;
            },
            Err(errno) => return Err(errno.into()),
        }
        if !ready[1].revents().is_empty() {
            return Ok(());
        }
        if ready[0].revents().intersects(LISTENER_FAILED) {
            return Err(io::Error::other(LISTENER_HUNG_UP));
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
            Err(error) if Errno::from_io_error(&error).is_some_and(passes) => {
                thread::sleep(ACCEPT_BACKOFF);
                continue;
            },
            Err(error) => return Err(error),
        };
        let Some(reader) = stream
            .set_nonblocking(false)
            .and_then(|()| stream.try_clone())
            .ok()
        else {
            continue;
        };
        let client = counts.next_client();
        if !deliver(events, HolderEvent::Connected { client, stream }, woken) {
            return Ok(());
        }
        spawn_reader(client, reader, events.clone());
    }
}

/// Whether a poll or accept that failed with `errno` may work when tried
/// again: memory, buffers or descriptors ran short, or one connection went
/// away before it was taken.
fn passes(errno: Errno) -> bool {
    matches!(
        errno,
        Errno::AGAIN
            | Errno::NOMEM
            | Errno::NOBUFS
            | Errno::MFILE
            | Errno::NFILE
            | Errno::CONNABORTED
            | Errno::PROTO
    )
}

/// Hands `event` to the holder loop, waiting while its inbox is full, and
/// gives up once `woken` asks the thread to stop, since a retired acceptor
/// has nothing more to tell the loop. Returns whether the event was handed
/// over.
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

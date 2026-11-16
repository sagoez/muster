use std::{
    io::{self, Write},
    net::Shutdown,
    os::unix::net::{UnixListener, UnixStream},
    sync::Arc,
    thread,
    time::Duration,
};

use crossbeam_channel::Sender;
use muster_protocol::holder::ClientMessage;
use rustix::{
    event::{PollFd, PollFlags, poll},
    io::Errno,
};

use crate::{
    clients::client_id::ClientId,
    serve::{holder_event::HolderEvent, socket::acceptor_counts::AcceptorCounts},
};

/// Pause after a poll or an accept fails for a reason that passes, such as
/// running out of memory or file descriptors, so a listener that stays
/// readable is not spun on.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
/// Why an acceptor a test refused did not start.
#[cfg(test)]
const REFUSED_START: &str = "a test refused the acceptor's start";
/// The byte that wakes an acceptor to stop.
const WAKE: u8 = 0;

/// One thread accepting connections on the holder's listener and handing them
/// to the holder loop. Dropping it wakes the thread to stop without waiting
/// for it, so the loop never blocks on a join; the thread first takes the
/// connections already waiting on the listener, then closes its end of the
/// listener as it returns, which takes a moment.
pub(super) struct Acceptor {
    /// The end that wakes the thread to stop.
    wake: UnixStream,
    /// The listener, shared with the thread until it returns, and held so the
    /// socket's file stays the one bound, its inode never freed, for as long
    /// as the acceptor is kept, even should the thread have ended.
    _listener: Arc<UnixListener>,
}

impl Acceptor {
    /// Accepts connections on `listener` on a thread of its own, numbering
    /// each from `counts`, which every acceptor of the holder shares.
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
        let listener = Arc::new(listener);
        let accepting = Arc::clone(&listener);
        let counted = Arc::clone(counts);
        thread::Builder::new().spawn(move || {
            accept_until_woken(&accepting, &woken, &events, &counted);
        })?;
        Ok(Self {
            wake,
            _listener: listener,
        })
    }

    /// The listener the acceptor serves, shared, so a test sees when the
    /// thread lets go of it.
    #[cfg(test)]
    pub(super) fn listener(&self) -> Arc<UnixListener> {
        Arc::clone(&self._listener)
    }
}

impl Drop for Acceptor {
    /// Wakes the thread to stop, never waiting for it.
    fn drop(&mut self) {
        let _ = self.wake.write_all(&[WAKE]);
        let _ = self.wake.shutdown(Shutdown::Write);
    }
}

/// Accepts connections on `listener` until `woken` is readable, the holder
/// loop is gone or the listener fails for good, giving each a reader thread
/// that forwards its messages to the loop. Once woken it still takes every
/// connection already waiting, so a client that connected just before the
/// socket was bound again is served rather than cut off. A poll or accept
/// failure that passes is waited out and tried again.
fn accept_until_woken(
    listener: &UnixListener,
    woken: &UnixStream,
    events: &Sender<HolderEvent>,
    counts: &AcceptorCounts,
) {
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
            Err(_) => return,
        }
        let stopping = !ready[1].revents().is_empty();
        if !accept_waiting(listener, events, counts) || stopping {
            return;
        }
    }
}

/// Accepts every connection waiting on `listener` and hands each to the
/// holder loop. Returns whether to go on accepting: not once the loop is
/// gone or the listener failed for good.
fn accept_waiting(
    listener: &UnixListener,
    events: &Sender<HolderEvent>,
    counts: &AcceptorCounts,
) -> bool {
    loop {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return true,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if Errno::from_io_error(&error).is_some_and(passes) => {
                thread::sleep(ACCEPT_BACKOFF);
                return true;
            },
            Err(_) => return false,
        };
        let Some(reader) = stream
            .set_nonblocking(false)
            .and_then(|()| stream.try_clone())
            .ok()
        else {
            continue;
        };
        let client = counts.next_client();
        if events
            .send(HolderEvent::Connected { client, stream })
            .is_err()
        {
            return false;
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

#[cfg(test)]
mod tests {
    use crossbeam_channel::unbounded;
    use tempfile::TempDir;

    use super::*;

    /// File name of the test listener's socket.
    const SOCKET_FILE: &str = "a.sock";

    /// An acceptor woken while a client waits on its listener still takes
    /// that client and hands it to the loop, rather than cut it off.
    #[test]
    fn a_woken_acceptor_takes_the_clients_already_waiting() {
        let scratch = TempDir::new().unwrap();
        let listener = UnixListener::bind(scratch.path().join(SOCKET_FILE)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let _client = UnixStream::connect(scratch.path().join(SOCKET_FILE)).unwrap();
        let (wake, woken) = UnixStream::pair().unwrap();
        (&wake).write_all(&[WAKE]).unwrap();
        let (events, inbox) = unbounded();

        accept_until_woken(&listener, &woken, &events, &AcceptorCounts::default());

        assert!(matches!(
            inbox.try_recv(),
            Ok(HolderEvent::Connected { .. })
        ));
    }
}

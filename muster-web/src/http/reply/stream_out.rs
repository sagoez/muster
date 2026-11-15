use std::{
    io::{ErrorKind, Write},
    net::TcpStream,
};

/// Where a stream writes: its client, which it can ask whether it hung up
/// without writing to it.
pub trait StreamOut: Write {
    /// Whether the client closed its end or the connection broke, looked
    /// at without waiting and without taking anything it sent.
    fn hung_up(&mut self) -> bool;
}

impl StreamOut for TcpStream {
    /// Peeks at what the client sent, without blocking: an end of stream is
    /// a client that closed its tab, a byte is one still there, nothing to
    /// read is one still there and quiet.
    fn hung_up(&mut self) -> bool {
        if self.set_nonblocking(true).is_err() {
            return true;
        }
        let mut byte = [0; 1];
        let gone = match self.peek(&mut byte) {
            Ok(read) => read == 0,
            Err(error) => !matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted),
        };
        self.set_nonblocking(false).is_err() || gone
    }
}

#[cfg(test)]
impl StreamOut for Vec<u8> {
    /// A buffer never hangs up.
    fn hung_up(&mut self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, TcpListener};

    use muster_os::test_wait::wait_until;

    use super::*;

    /// A connection whose client is still there has not hung up; once the
    /// client closes it has.
    #[test]
    fn a_closed_client_has_hung_up() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut served, _) = listener.accept().unwrap();

        assert!(!served.hung_up());
        drop(client);

        wait_until("the closed client to be seen", || served.hung_up());
    }
}

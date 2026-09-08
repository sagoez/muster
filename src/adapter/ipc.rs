use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Serialize, de::DeserializeOwned};

use crate::adapter::config::state_dir_path;

/// Subdirectory of the state dir holding per-project workspace sockets.
const SOCKET_DIR: &str = "ipc";
/// Suffix of a workspace IPC socket file.
const SOCKET_SUFFIX: &str = ".sock";
/// Longest a peer may take to send its request line, or to read the response,
/// before the connection is abandoned. Bounds a client that connects and then
/// stalls, so it cannot hold a server thread forever.
const PEER_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest request accepted, in bytes. Caps server-side allocation so one peer
/// cannot force unbounded memory with an endless line.
const MAX_REQUEST_BYTES: u64 = 1 << 20;

/// The unix-socket path a running workspace for `project` listens on, and that an
/// MCP client connects to. The absolutized project path is hashed so the socket
/// name stays within the platform's socket-path length limit and is stable for a
/// given project across processes.
///
/// A unix socket path is capped by `sun_path` (104 bytes on macOS, 108 on Linux),
/// which is why the project is hashed to 16 characters rather than embedded. If
/// the state directory itself is long enough to breach that cap, `bind` fails and
/// live workspace tools stay unavailable; coordination is unaffected.
#[must_use]
pub fn socket_path(project: &Path) -> Option<PathBuf> {
    let mut hasher = DefaultHasher::new();
    project.hash(&mut hasher);
    let name = format!("{:016x}{SOCKET_SUFFIX}", hasher.finish());
    state_dir_path(SOCKET_DIR).map(|dir| dir.join(name))
}

/// Binds a listener at `path`, creating its parent directory.
///
/// A socket file that another live workspace is serving is never stolen: the path
/// is probed first, and a successful connect means an owner exists. Only a stale
/// file left by a crashed run (nothing accepting) is cleared and rebound.
///
/// # Errors
/// Returns [`io::ErrorKind::AddrInUse`] when another workspace already serves this
/// project, or an [`io::Error`] if the directory or socket cannot be created.
pub fn bind(path: &Path) -> io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match UnixStream::connect(path) {
        // Someone is accepting: this project already has a live workspace.
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "another muster workspace is already serving this project",
            ));
        },
        // Nothing there at all: a clean first bind.
        Err(error) if error.kind() == io::ErrorKind::NotFound => {},
        // The file exists but nothing accepts: stale, safe to replace.
        Err(_) => match std::fs::remove_file(path) {
            Ok(()) => {},
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        },
    }
    UnixListener::bind(path)
}

/// Removes a workspace socket file, ignoring an already-absent path. Called on
/// clean shutdown so sockets do not accumulate in the state directory.
pub fn unlink(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Sends `request` to the workspace socket at `path` and returns its response,
/// one newline-delimited JSON message each way.
///
/// # Errors
/// Returns an [`io::Error`] if the socket cannot be reached (no running
/// workspace) or the exchange fails.
pub fn request<Req, Resp>(path: &Path, request: &Req) -> io::Result<Resp>
where
    Req: Serialize,
    Resp: DeserializeOwned,
{
    let stream = UnixStream::connect(path)?;
    let encoded = serde_json::to_string(request).map_err(io::Error::other)?;
    (&stream).write_all(encoded.as_bytes())?;
    (&stream).write_all(b"\n")?;
    (&stream).flush()?;
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    serde_json::from_str(line.trim_end()).map_err(io::Error::other)
}

/// Reads one request from `stream`, hands it to `handle`, and writes the response
/// back as one newline-delimited JSON message.
///
/// # Errors
/// Returns an [`io::Error`] if the request cannot be read or parsed or the
/// response cannot be written.
pub fn serve_connection<Req, Resp>(
    stream: UnixStream,
    handle: impl FnOnce(Req) -> Resp,
) -> io::Result<()>
where
    Req: DeserializeOwned,
    Resp: Serialize,
{
    // A peer that connects and then stalls must not hold this thread forever,
    // and must not be able to force unbounded allocation with an endless line.
    stream.set_read_timeout(Some(PEER_TIMEOUT))?;
    stream.set_write_timeout(Some(PEER_TIMEOUT))?;
    let mut reader = BufReader::new((&stream).take(MAX_REQUEST_BYTES));
    let mut line = String::new();
    let read = reader.read_line(&mut line)?;
    if read == 0 || line.trim().is_empty() {
        return Ok(());
    }
    if !line.ends_with('\n') {
        return Err(io::Error::other(
            "request exceeded the maximum frame size before a newline",
        ));
    }
    let request: Req = serde_json::from_str(line.trim_end()).map_err(io::Error::other)?;
    let response = handle(request);
    let encoded = serde_json::to_string(&response).map_err(io::Error::other)?;
    (&stream).write_all(encoded.as_bytes())?;
    (&stream).write_all(b"\n")?;
    (&stream).flush()
}

#[cfg(test)]
mod tests {
    use std::thread;

    use serde::Deserialize;

    use super::*;

    #[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
    enum Ping {
        Ping,
    }

    #[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
    enum Pong {
        Pong(u32),
    }

    /// A socket path short enough for every platform's `sun_path` cap. macOS
    /// resolves `env::temp_dir()` to a long `/var/folders/...` path that breaches
    /// the 104-byte limit, so these tests bind under `/tmp` directly.
    fn short_socket_path() -> PathBuf {
        let unique = uuid::Uuid::new_v4().simple().to_string();
        Path::new("/tmp").join(format!("mstr-{}.sock", &unique[..8]))
    }

    /// A socket path is stable for a project and differs between projects.
    #[test]
    fn socket_paths_are_stable_and_project_scoped() {
        let one = socket_path(Path::new("/repo/a/muster.yml"));
        let two = socket_path(Path::new("/repo/a/muster.yml"));
        let other = socket_path(Path::new("/repo/b/muster.yml"));
        assert_eq!(one, two, "same project resolves to the same socket");
        assert_ne!(one, other, "different projects get different sockets");
    }

    /// A request round-trips through the listener to a handler and back.
    #[test]
    fn a_request_round_trips_through_the_socket() {
        let path = short_socket_path();
        let listener = bind(&path).unwrap();

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_connection(stream, |request: Ping| {
                assert_eq!(request, Ping::Ping);
                Pong::Pong(42)
            })
            .unwrap();
        });

        let response: Pong = request(&path, &Ping::Ping).unwrap();
        assert_eq!(response, Pong::Pong(42));
        server.join().unwrap();
        unlink(&path);
    }

    /// A live workspace's socket is never stolen by a second instance, but a
    /// stale file left by a crashed run is reclaimed.
    #[test]
    fn bind_refuses_a_live_socket_and_reclaims_a_stale_one() {
        let path = short_socket_path();
        let live = bind(&path).unwrap();
        let stolen = bind(&path);
        assert_eq!(
            stolen.unwrap_err().kind(),
            io::ErrorKind::AddrInUse,
            "a second instance must not unlink the first's socket"
        );

        // Drop the listener: the file remains but nothing accepts, so the next
        // bind may reclaim it.
        drop(live);
        assert!(path.exists(), "the stale socket file is still on disk");
        bind(&path).expect("a stale socket is reclaimed");
        unlink(&path);
    }

    /// A peer that floods bytes without a newline is cut off at the frame cap
    /// instead of growing the server's memory without bound.
    #[test]
    fn an_oversized_request_is_refused() {
        let path = short_socket_path();
        let listener = bind(&path).unwrap();

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_connection(stream, |_: Ping| Pong::Pong(0))
        });

        let stream = UnixStream::connect(&path).unwrap();
        let flood = vec![b'x'; (MAX_REQUEST_BYTES as usize) + 1024];
        // The peer never sends a newline; the write may fail once the server
        // gives up, which is itself the cap working.
        let _ = (&stream).write_all(&flood);
        assert!(
            server.join().unwrap().is_err(),
            "the server refuses a frame that exceeds the cap"
        );
        unlink(&path);
    }

    /// Connecting when nothing listens is an error the caller reads as "no
    /// running workspace".
    #[test]
    fn connecting_with_no_listener_errors() {
        let path =
            std::env::temp_dir().join(format!("muster-absent-{}.sock", uuid::Uuid::new_v4()));
        let result: io::Result<Pong> = request(&path, &Ping::Ping);
        assert!(result.is_err());
    }
}

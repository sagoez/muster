//! `muster view` end to end: the built binary serving its page on loopback
//! over isolated state, answering only requests under its token's path,
//! never starting the service, and ending cleanly on Ctrl-C.

use std::{
    fs,
    net::{Ipv4Addr, TcpListener},
    thread,
    time::{Duration, Instant},
};

use muster_protocol::service::endpoint::SERVICE_SUBCOMMAND;
use muster_web::{exchange, get};

use crate::common::{Scratch, ViewSession, entry, processes_under, stack};

/// What the view prints after its address.
const STOP_HINT: &str = "Press Ctrl-C to stop.";
/// The page's Content Security Policy, as the spec writes it.
const POLICY: &str = "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'";
/// How long the view runs before a test checks it started nothing.
const SETTLE: Duration = Duration::from_secs(3);
/// The pause between the page loads a settling test makes.
const LOOK_PAUSE: Duration = Duration::from_millis(500);

/// The view answers only under its token's path: the bare root, a wrong
/// token and a cookie are forbidden, the printed address opens the shell
/// with the exact policy and this version's entry module, and a foreign
/// host or a POST is refused.
#[test]
fn the_view_answers_only_with_its_token() {
    let mut scratch = Scratch::new();
    let view = ViewSession::start(&mut scratch, &[]);
    let port = view.port();
    let root = format!("/v/{}/", view.token());

    assert_eq!(view.next_line(), STOP_HINT);
    assert_eq!(view.token().len(), 64, "{}", view.url());
    assert_eq!(view.url(), format!("http://127.0.0.1:{port}{root}"));

    let cookie = format!("muster-view-{port}={}", view.token());
    assert_eq!(get(port, "/", &[]).status(), &403);
    assert_eq!(get(port, "/", &[("Cookie", &cookie)]).status(), &403);
    assert_eq!(
        get(port, &format!("/v/{}/", "0".repeat(64)), &[]).status(),
        &403
    );

    let shell = get(port, &root, &[]);
    assert_eq!(shell.status(), &200);
    assert_eq!(shell.header("Content-Security-Policy"), Some(POLICY));
    assert_eq!(shell.header("Set-Cookie"), None);
    let entry_module = format!("assets/v{}/js/app.js", env!("CARGO_PKG_VERSION"));
    assert!(shell.text().contains(&entry_module), "{}", shell.text());
    assert_eq!(
        get(port, &format!("{root}{entry_module}"), &[]).status(),
        &200
    );

    let foreign = exchange(
        port,
        format!("GET {root} HTTP/1.1\r\nHost: evil.example:{port}\r\n\r\n").as_bytes(),
    );
    assert_eq!(foreign.status(), &403);

    let post = exchange(
        port,
        format!("POST {root} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\n\r\n")
            .as_bytes(),
    );
    assert_eq!(post.status(), &405);
}

/// The view listens on the port it is given.
#[test]
fn the_view_listens_on_the_port_it_is_given() {
    let mut scratch = Scratch::new();
    let free = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();

    let view = ViewSession::start(&mut scratch, &["--port", &free.to_string()]);

    assert_eq!(view.port(), free);
    assert_eq!(get(free, "/", &[]).status(), &403);
}

/// Looking never starts the service: while a page loads the view again and
/// again against a registered project, no service runs and no socket exists.
#[test]
fn the_view_never_starts_the_service() {
    let mut scratch = Scratch::new();
    let project = scratch.path().join("shop");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("muster.yml"),
        stack(&[entry("api", "sleep 600", true)]),
    )
    .unwrap();
    let import = scratch.muster(&["-p", project.to_str().unwrap(), "import"]);
    assert!(import.status.success(), "{import:?}");
    let mut view = ViewSession::start(&mut scratch, &[]);
    let root = format!("/v/{}/", view.token());

    let started = Instant::now();
    while started.elapsed() < SETTLE {
        assert_eq!(get(view.port(), &root, &[]).status(), &200);
        thread::sleep(LOOK_PAUSE);
    }

    assert!(view.running());
    assert!(processes_under(SERVICE_SUBCOMMAND, scratch.root()).is_empty());
    assert!(!scratch.socket().exists());
}

/// Ctrl-C ends the view cleanly.
#[test]
fn ctrl_c_ends_the_view() {
    let mut scratch = Scratch::new();
    let mut view = ViewSession::start(&mut scratch, &[]);

    let status = view.interrupt();

    assert!(status.success(), "{status:?}");
}

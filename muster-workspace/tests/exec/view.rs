//! `muster view` end to end: the built binary serving its page on loopback
//! over isolated state, answering only requests that carry its token, and
//! never starting the service.

use std::{
    fs,
    net::{Ipv4Addr, TcpListener},
    thread,
    time::Duration,
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

/// The view answers only with its token: the bare page is forbidden, the
/// address it printed sets its cookie, the cookie opens the shell with the
/// exact policy and this version's entry module, and a foreign host or a
/// POST is refused.
#[test]
fn the_view_answers_only_with_its_token() {
    let mut scratch = Scratch::new();
    let view = ViewSession::start(&mut scratch, &[]);
    let port = view.port();

    assert_eq!(view.next_line(), STOP_HINT);
    assert_eq!(view.token().len(), 64, "{}", view.url());

    assert_eq!(get(port, "/", &[]).status(), &403);

    let first = get(port, &format!("/?key={}", view.token()), &[]);
    assert_eq!(first.status(), &303);
    let cookie = format!("muster-view-{port}={}", view.token());
    assert_eq!(
        first.header("Set-Cookie"),
        Some(format!("{cookie}; HttpOnly; SameSite=Strict; Path=/").as_str())
    );

    let shell = get(port, "/", &[("Cookie", &cookie)]);
    assert_eq!(shell.status(), &200);
    assert_eq!(shell.header("Content-Security-Policy"), Some(POLICY));
    let entry_module = format!("/assets/v{}/js/app.js", env!("CARGO_PKG_VERSION"));
    assert!(shell.text().contains(&entry_module), "{}", shell.text());

    let foreign = exchange(
        port,
        format!("GET / HTTP/1.1\r\nHost: evil.example:{port}\r\nCookie: {cookie}\r\n\r\n")
            .as_bytes(),
    );
    assert_eq!(foreign.status(), &403);

    let post = exchange(
        port,
        format!(
            "POST / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: {cookie}\r\nContent-Length: 0\r\n\r\n"
        )
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

/// Looking never starts the service: after a while against a registered
/// project, no service runs and no socket exists.
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

    thread::sleep(SETTLE);

    assert!(view.running());
    assert!(processes_under(SERVICE_SUBCOMMAND, scratch.root()).is_empty());
    assert!(!scratch.socket().exists());
}

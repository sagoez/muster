use std::{borrow::Cow, time::Instant};

use getset::Getters;
use typed_builder::TypedBuilder;

use super::{
    asset::asset,
    event_stream::EventStream,
    log_answer::{LogAnswer, UNKNOWN_PROCESS, UNREADABLE_LOG},
    log_tail::read_tail,
    log_target::LogTarget,
    route::Route,
    shell::{SHELL_MEDIA_TYPE, shell},
};
use crate::{
    http::{Admission, Answer, CachePolicy, Gate, Request, Responder, Response, Status},
    live::HubHandle,
};

/// The view's pages behind its gate: the shell, this version's assets, the
/// stream and the log tails, all below the token's path.
#[derive(Clone, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Site {
    /// What decides which requests are served.
    gate: Gate,
    /// The hub the stream and the log tails are asked of.
    hub: HubHandle,
}

impl Site {
    /// The answer to an admitted request for `path`, below the token's path,
    /// with its `query`.
    fn route(&self, path: &str, query: Option<&str>) -> Answer {
        match Route::parse(path, query) {
            Route::Shell => Response::ok(
                SHELL_MEDIA_TYPE,
                CachePolicy::NoStore,
                Cow::Owned(shell().into_bytes()),
            )
            .into(),
            Route::Asset(path) => asset(&path)
                .map_or_else(
                    || Response::refusal(Status::NotFound),
                    |(kind, body)| {
                        Response::ok(
                            kind.media_type(),
                            CachePolicy::Immutable,
                            Cow::Borrowed(body),
                        )
                    },
                )
                .into(),
            Route::Events => self.events(),
            Route::Log(target) => self.log_tail(&target).into(),
            Route::NotFound => Response::refusal(Status::NotFound).into(),
        }
    }

    /// The live stream for one more page, its snapshot first; 503 when too
    /// many are open or the hub has ended.
    fn events(&self) -> Answer {
        match self.hub.subscribe() {
            Ok(Some(subscription)) => Answer::Stream(Box::new(EventStream::new(subscription))),
            Ok(None) | Err(_) => Response::refusal(Status::ServiceUnavailable).into(),
        }
    }

    /// The log tail of the process `target` names, located by the hub and
    /// read on this connection's thread: 404 for a process not on the
    /// floor, 503 once the hub has ended.
    fn log_tail(&self, target: &LogTarget) -> Response {
        match self.hub.locate_log(target.project(), target.name()) {
            Err(_) => Response::refusal(Status::ServiceUnavailable),
            Ok(None) => LogAnswer::Error {
                error: UNKNOWN_PROCESS,
            }
            .response(Status::NotFound),
            Ok(Some(log)) => match read_tail(&log) {
                Ok(lines) => LogAnswer::Lines { lines }.response(Status::Ok),
                Err(_) => LogAnswer::Error {
                    error: UNREADABLE_LOG,
                }
                .response(Status::InternalServerError),
            },
        }
    }
}

impl Responder for Site {
    /// Refuses what the gate refuses, sends a browser opened with the
    /// one-time code on to the token's path, and routes the rest.
    fn respond(&self, request: &Request) -> Answer {
        match self.gate.admit(request, Instant::now()) {
            Admission::Refused(status) => Response::refusal(status).into(),
            Admission::Redirect(location) => Response::see_other(location).into(),
            Admission::Admitted(below) => self.route(&below, request.query().as_deref()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, sync::Arc};

    use muster_domain::value::ProcessName;
    use muster_os::{test_scratch::short_scratch, test_wait::wait_until};
    use muster_protocol::holder::HolderDirs;
    use serde_json::{Value, json};

    use super::*;
    use crate::{
        floor::{
            Floor, ProjectsReading,
            fixture::{floor, project_reading, root},
        },
        http::{
            CONTENT_SECURITY_POLICY, EventReader, HttpReply, HttpServer, ViewToken, exchange, get,
        },
        live::{hub_event::HubEvent, subscribers::MAX_STREAMS},
        routes::testing::live_site,
    };

    /// A started server on a free port answered by a fresh site over the
    /// fixture floor; returns its port and its token's text.
    fn started() -> (u16, String) {
        let (port, gate, _) = started_over(floor());
        (port, gate.token().secret().to_string())
    }

    /// A started server on a free port answered by a fresh site whose hub
    /// owns `floor`; returns its port, its gate, which shares the site's
    /// one-time code, and its hub.
    fn started_over(floor: Floor) -> (u16, Gate, HubHandle) {
        let server = HttpServer::bind(None).unwrap();
        let port = server.port().unwrap();
        let gate = Gate::builder()
            .port(port)
            .token(ViewToken::generate().unwrap())
            .build();
        let (site, hub) = live_site(gate.clone(), floor);
        server.start(Arc::new(site)).unwrap();
        (port.into_inner(), gate, hub)
    }

    /// The JSON body of `reply`.
    fn json_body(reply: &HttpReply) -> Value {
        serde_json::from_slice(reply.body()).unwrap()
    }

    /// The browser's opening address answers a hardened 303 to the token's
    /// path once, sending no referrer on; its second use is forbidden.
    #[test]
    fn the_opening_address_redirects_once() {
        let (port, gate, _) = started_over(floor());
        let url = gate.open_url(Instant::now()).unwrap();
        let path = url.replace(&format!("http://127.0.0.1:{port}"), "");

        let first = get(port, &path, &[]);

        assert_eq!(first.status(), &303);
        assert_eq!(
            first.header("Location"),
            Some(format!("/v/{}/", gate.token().secret()).as_str())
        );
        assert_hardened(&first);
        assert_eq!(first.header("Cache-Control"), Some("no-store"));
        assert_eq!(get(port, &path, &[]).status(), &403);
    }

    /// Asserts `reply` carries the policy and the hardening headers.
    fn assert_hardened(reply: &HttpReply) {
        assert_eq!(
            reply.header("Content-Security-Policy"),
            Some(CONTENT_SECURITY_POLICY)
        );
        assert_eq!(reply.header("X-Content-Type-Options"), Some("nosniff"));
        assert_eq!(reply.header("Referrer-Policy"), Some("no-referrer"));
        assert_eq!(reply.header("X-Frame-Options"), Some("DENY"));
    }

    /// Without the token's path everything is forbidden, a cookie or query
    /// key included; under it the shell, this version's assets and every
    /// other route are reached, and no response sets a cookie.
    #[test]
    fn only_the_token_path_opens_the_view() {
        let (port, secret) = started();
        let root = format!("/v/{secret}/");
        let carried = format!("muster-view-{port}={secret}");

        for path in [
            "/".to_string(),
            format!("/?key={secret}"),
            format!("/v/{}/", "0".repeat(64)),
            format!("/v/{secret}"),
            "/events".to_string(),
            "/api/log?project=shop&name=web".to_string(),
            format!("/assets/v{}/js/app.js", env!("CARGO_PKG_VERSION")),
        ] {
            let refused = get(port, &path, &[("Cookie", &carried)]);
            assert_eq!(refused.status(), &403, "{path}");
            assert_hardened(&refused);
            assert_eq!(refused.header("Cache-Control"), Some("no-store"));
        }

        let shell = get(port, &root, &[]);
        assert_eq!(shell.status(), &200);
        assert_hardened(&shell);
        assert_eq!(shell.header("Cache-Control"), Some("no-store"));
        assert_eq!(shell.header("Set-Cookie"), None);
        assert_eq!(
            shell.header("Content-Type"),
            Some("text/html; charset=utf-8")
        );
        let entry = format!("assets/v{}/js/app.js", env!("CARGO_PKG_VERSION"));
        assert!(shell.text().contains(&entry), "{}", shell.text());

        let module = get(port, &format!("{root}{entry}"), &[]);
        assert_eq!(module.status(), &200);
        assert_hardened(&module);
        assert_eq!(
            module.header("Cache-Control"),
            Some("public, max-age=31536000, immutable")
        );
        assert_eq!(
            module.header("Content-Type"),
            Some("text/javascript; charset=utf-8")
        );

        let nowhere = get(port, &format!("{root}nowhere"), &[]);
        assert_eq!(nowhere.status(), &404);
        assert_hardened(&nowhere);
        let events = EventReader::open(port, &format!("{root}events"));
        assert_eq!(*events.status(), 200);
        assert_eq!(
            events.header("Content-Security-Policy"),
            Some(CONTENT_SECURITY_POLICY)
        );
        assert_eq!(events.header("Set-Cookie"), None);
    }

    /// The stream is a close-delimited event stream that is never kept,
    /// tells the page to reconnect after a second, and opens with a
    /// snapshot of the floor.
    #[test]
    fn the_stream_opens_with_a_snapshot() {
        let (port, gate, hub) = started_over(floor());
        hub.send(HubEvent::Projects(ProjectsReading::Read(vec![
            project_reading("shop", &["web", "worker"]),
        ])));

        let mut events = EventReader::open(port, &format!("/v/{}/events", gate.token().secret()));

        assert_eq!(*events.status(), 200);
        assert_eq!(events.header("Content-Type"), Some("text/event-stream"));
        assert_eq!(events.header("Cache-Control"), Some("no-store"));
        assert_eq!(events.header("Content-Length"), None);
        assert_eq!(events.header("X-Frame-Options"), Some("DENY"));
        assert_eq!(events.next_block(), Some(vec!["retry: 1000".to_string()]));
        let snapshot = events.next_event().unwrap();
        assert_eq!(snapshot.kind(), "snapshot");
        assert_eq!(snapshot.data()["health"], "absent");
        let names: Vec<&Value> = snapshot.data()["projects"][0]["processes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|process| &process["name"])
            .collect();
        assert_eq!(names, [&json!("web"), &json!("worker")]);
    }

    /// Past the most streams one more is answered 503, as is every stream
    /// and log tail once the hub has ended.
    #[test]
    fn too_many_streams_or_no_hub_is_503() {
        let (port, gate, hub) = started_over(floor());
        let root = format!("/v/{}/", gate.token().secret());
        let open: Vec<EventReader> = (0..MAX_STREAMS)
            .map(|_| EventReader::open(port, &format!("{root}events")))
            .collect();
        assert!(open.iter().all(|stream| *stream.status() == 200));

        let over = get(port, &format!("{root}events"), &[]);
        hub.stop();

        assert_eq!(over.status(), &503);
        assert_hardened(&over);
        wait_until("the hub to end", || {
            get(port, &format!("{root}api/log?project=shop&name=web"), &[]).status() == &503
        });
        assert_eq!(get(port, &format!("{root}events"), &[]).status(), &503);
    }

    /// A log tail is found only for a process on the floor, by the names its
    /// query writes, each decoded once: its last lines, none before it logs,
    /// a process named `..` as itself, and a JSON 404 for any other name.
    #[test]
    fn a_log_tail_is_read_only_for_a_process_on_the_floor() {
        let scratch = short_scratch("mv-");
        let dirs = HolderDirs::under(scratch.path());
        let (port, gate, hub) = started_over(Floor::new(PathBuf::from("/"), None, dirs.clone()));
        let root_path = format!("/v/{}/api/log?", gate.token().secret());
        hub.send(HubEvent::Projects(ProjectsReading::Read(vec![
            project_reading("shop", &["dev server", "worker", ".."]),
        ])));
        for (name, text) in [
            ("dev server", "booting\n\x1b[32mready\x1b[0m\n"),
            ("..", "dots\n"),
        ] {
            let log = dirs.process_log(&root("shop"), &ProcessName::try_new(name).unwrap());
            fs::create_dir_all(log.parent().unwrap()).unwrap();
            fs::write(&log, text).unwrap();
        }

        let logged = get(
            port,
            &format!("{root_path}project=shop&name=dev%20server"),
            &[],
        );
        let silent = get(port, &format!("{root_path}name=worker&project=shop"), &[]);
        let dotted = get(port, &format!("{root_path}project=shop&name=.."), &[]);

        assert_eq!(logged.status(), &200);
        assert_eq!(logged.header("Content-Type"), Some("application/json"));
        assert_eq!(logged.header("Cache-Control"), Some("no-store"));
        assert_hardened(&logged);
        assert_eq!(json_body(&logged), json!({"lines": ["booting", "ready"]}));
        assert_eq!(json_body(&silent), json!({"lines": []}));
        assert_eq!(json_body(&dotted), json!({"lines": ["dots"]}));
        for unknown in [
            "project=shop&name=web",
            "project=blog&name=worker",
            "project=shop&name=..%2Fworker",
            "project=shop&name=dev%2520server",
        ] {
            let refused = get(port, &format!("{root_path}{unknown}"), &[]);
            assert_eq!(refused.status(), &404, "{unknown}");
            assert_eq!(
                json_body(&refused),
                json!({"error": "unknown process"}),
                "{unknown}"
            );
        }
    }

    /// A foreign `Host` or `Origin` is refused even on the token's path; the
    /// page's own origin is admitted.
    #[test]
    fn a_foreign_host_or_origin_is_refused() {
        let (port, secret) = started();
        let root = format!("/v/{secret}/");
        let with_host = |host: &str| {
            exchange(
                port,
                format!("GET {root} HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes(),
            )
        };

        assert_eq!(with_host(&format!("evil.example:{port}")).status(), &403);
        assert_eq!(with_host(&format!("127.0.0.1:{}", port + 1)).status(), &403);
        assert_eq!(with_host(&format!("localhost:{port}")).status(), &200);
        assert_eq!(
            get(port, &root, &[("Origin", "http://evil.example")]).status(),
            &403
        );
        assert_eq!(
            get(port, &root, &[(
                "Origin",
                &format!("http://localhost:{port}")
            )])
            .status(),
            &200
        );
    }
}

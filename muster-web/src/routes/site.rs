use std::borrow::Cow;

use getset::Getters;
use typed_builder::TypedBuilder;

use super::{
    asset::asset,
    route::Route,
    shell::{SHELL_MEDIA_TYPE, shell},
};
use crate::http::{Admission, CachePolicy, Gate, Request, Responder, Response, Status};

/// The view's pages behind its gate: the shell, this version's assets, the
/// stream and the log tails, all below the token's path.
#[derive(Clone, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Site {
    /// What decides which requests are served.
    gate: Gate,
}

impl Site {
    /// The answer to an admitted request for `path`, below the token's path.
    /// The stream and the log tails are not served yet.
    fn route(path: &str) -> Response {
        match Route::parse(path) {
            Route::Shell => Response::ok(
                SHELL_MEDIA_TYPE,
                CachePolicy::NoStore,
                Cow::Owned(shell().into_bytes()),
            ),
            Route::Asset(path) => asset(&path).map_or_else(
                || Response::refusal(Status::NotFound),
                |(kind, body)| {
                    Response::ok(
                        kind.media_type(),
                        CachePolicy::Immutable,
                        Cow::Borrowed(body),
                    )
                },
            ),
            Route::Events | Route::Log(_) | Route::NotFound => Response::refusal(Status::NotFound),
        }
    }
}

impl Responder for Site {
    /// Refuses what the gate refuses and routes the rest.
    fn respond(&self, request: &Request) -> Response {
        match self.gate.admit(request) {
            Admission::Refused(status) => Response::refusal(status),
            Admission::Admitted(below) => Self::route(&below),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::http::{CONTENT_SECURITY_POLICY, HttpReply, HttpServer, ViewToken, exchange, get};

    /// A started server on a free port answered by a fresh site; returns its
    /// port and its token's text.
    fn started() -> (u16, String) {
        let server = HttpServer::bind(None).unwrap();
        let port = server.port().unwrap();
        let token = ViewToken::generate().unwrap();
        let secret = token.secret().to_string();
        let site = Site::builder()
            .gate(Gate::builder().port(port).token(token).build())
            .build();
        server.start(Arc::new(site)).unwrap();
        (port.into_inner(), secret)
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

        for below in ["events", "api/log/shop/web", "nowhere"] {
            let admitted = get(port, &format!("{root}{below}"), &[]);
            assert_eq!(admitted.status(), &404, "{below}");
            assert_hardened(&admitted);
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

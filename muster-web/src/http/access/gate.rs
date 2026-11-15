use std::{io, sync::Arc, time::Instant};

use getset::Getters;
use muster_domain::value::LocalPort;
use typed_builder::TypedBuilder;

use super::{admission::Admission, one_time_code::OneTimeCode, token::ViewToken};
use crate::http::{connection::request::Request, reply::status::Status};

/// The only method the view serves.
pub const GET: &str = "GET";
/// What every admitted path starts with, before the token.
const CAPABILITY_PREFIX: &str = "/v/";
/// What the path the browser is opened at starts with, before its one-time
/// code.
const OPEN_PREFIX: &str = "/open/";
/// What separates a path's segments.
const SEGMENT_SEPARATOR: char = '/';
/// The hosts a request may name, each followed by `:<port>`.
const LOOPBACK_HOSTS: [&str; 2] = ["127.0.0.1", "localhost"];
/// The scheme an allowed `Origin` names.
const ORIGIN_SCHEME: &str = "http://";

/// Decides which requests the view serves: GET only, naming the view's own
/// loopback address, from no foreign origin, under the path that carries its
/// token. No cookie is used: a loopback cookie would reach every port on the
/// machine, the dev servers muster runs among them. The browser is opened at
/// `/open/<code>` instead, which redirects once to the token's path, so the
/// token never reaches a process's arguments.
#[derive(Clone, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Gate {
    /// The port the view listens on.
    port: LocalPort,
    /// The secret every admitted path carries.
    token: ViewToken,
    /// The one-time code the browser is opened with.
    #[builder(default)]
    codes: Arc<OneTimeCode>,
}

impl Gate {
    /// Admits, redirects or refuses `request` at `now`, checking in order:
    /// the method, the `Host`, the `Origin`, then the path. `/open/<code>`
    /// with the pending one-time code redirects to the token's path; the
    /// code is used up either way. Under `/v/<token>/`, a request is
    /// admitted with its path below that prefix. Anything else is
    /// forbidden.
    pub fn admit(&self, request: &Request, now: Instant) -> Admission {
        if request.method() != GET {
            return Admission::Refused(Status::MethodNotAllowed);
        }
        if !matches!(request.hosts().as_slice(), [host] if self.is_own_host(host)) {
            return Admission::Refused(Status::Forbidden);
        }
        if !request
            .origins()
            .iter()
            .all(|origin| self.is_own_origin(origin))
        {
            return Admission::Refused(Status::Forbidden);
        }
        if let Some(code) = request.path().strip_prefix(OPEN_PREFIX) {
            return if self.codes.redeem(code, now) {
                Admission::Redirect(self.token_path())
            } else {
                Admission::Refused(Status::Forbidden)
            };
        }
        request
            .path()
            .strip_prefix(CAPABILITY_PREFIX)
            .and_then(|rest| rest.split_once(SEGMENT_SEPARATOR))
            .filter(|(token, _)| self.token.matches(token))
            .map_or(Admission::Refused(Status::Forbidden), |(_, below)| {
                Admission::Admitted(format!("{SEGMENT_SEPARATOR}{below}"))
            })
    }

    /// The address that opens the view, carrying the token in its path.
    pub fn url(&self) -> String {
        format!("{}{}", self.origin(), self.token_path())
    }

    /// Mints a fresh one-time code at `now` and returns the address that
    /// opens the view with it, which never carries the token.
    ///
    /// # Errors
    /// Returns the failure to read the random source.
    pub fn open_url(&self, now: Instant) -> io::Result<String> {
        Ok(format!(
            "{}{OPEN_PREFIX}{}",
            self.origin(),
            self.codes.mint(now)?
        ))
    }

    /// The view's own origin, `http://127.0.0.1:<port>`.
    fn origin(&self) -> String {
        format!("{ORIGIN_SCHEME}{}:{}", LOOPBACK_HOSTS[0], self.port)
    }

    /// The path every admitted request starts with, `/v/<token>/`.
    fn token_path(&self) -> String {
        format!(
            "{CAPABILITY_PREFIX}{}{SEGMENT_SEPARATOR}",
            self.token.secret()
        )
    }

    /// Whether `host` is one of the loopback names with this view's port,
    /// which a page reached through a rebound DNS name never sends.
    fn is_own_host(&self, host: &str) -> bool {
        LOOPBACK_HOSTS
            .iter()
            .any(|name| host.eq_ignore_ascii_case(&format!("{name}:{}", self.port)))
    }

    /// Whether `origin` is this view's own page.
    fn is_own_origin(&self, origin: &str) -> bool {
        origin
            .get(..ORIGIN_SCHEME.len())
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case(ORIGIN_SCHEME))
            && self.is_own_host(&origin[ORIGIN_SCHEME.len()..])
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::http::access::one_time_code::CODE_LIFETIME;

    /// The port the gate under test listens on.
    const PORT: u16 = 4173;

    /// A gate on [`PORT`] with a fresh token.
    fn gate() -> Gate {
        Gate::builder()
            .port(LocalPort::try_new(PORT).unwrap())
            .token(ViewToken::generate().unwrap())
            .build()
    }

    impl Gate {
        /// [`Gate::admit`] at the current instant.
        fn admit_now(&self, request: &Request) -> Admission {
            self.admit(request, Instant::now())
        }
    }

    /// A GET for `path` naming `127.0.0.1:<PORT>`.
    fn request(path: &str) -> Request {
        Request::builder()
            .method(GET.to_string())
            .path(path.to_string())
            .hosts(vec![format!("127.0.0.1:{PORT}")])
            .build()
    }

    /// Only the token as the path's first segment admits, and the request
    /// keeps its path below it; anything else is forbidden.
    #[test]
    fn only_the_token_path_admits() {
        let gate = gate();
        let secret = gate.token().secret().to_string();
        let wrong = ViewToken::generate().unwrap();

        for (below, path) in [
            ("/", format!("/v/{secret}/")),
            ("/events", format!("/v/{secret}/events")),
            ("/api/log", format!("/v/{secret}/api/log")),
            ("/nope", format!("/v/{secret}/nope")),
        ] {
            assert_eq!(
                gate.admit_now(&request(&path)),
                Admission::Admitted(below.to_string()),
                "{path}"
            );
        }
        for path in [
            "/".to_string(),
            "/events".to_string(),
            format!("/v/{}/", wrong.secret()),
            format!("/v/{secret}"),
            format!("/v/{}/", &secret[1..]),
            format!("/v/{secret}x/"),
            format!("/x/{secret}/"),
            "/v//".to_string(),
        ] {
            assert_eq!(
                gate.admit_now(&request(&path)),
                Admission::Refused(Status::Forbidden),
                "{path}"
            );
        }
    }

    /// A foreign host or port in `Host`, a missing or repeated `Host`, or a
    /// foreign `Origin` is refused even on the token's path; the loopback
    /// names with the view's port are admitted.
    #[test]
    fn a_foreign_host_or_origin_is_refused() {
        let gate = gate();
        let path = format!("/v/{}/", gate.token().secret());
        let with = |hosts: Vec<String>, origins: Vec<String>| {
            Request::builder()
                .method(GET.to_string())
                .path(path.clone())
                .hosts(hosts)
                .origins(origins)
                .build()
        };
        let forbidden = Admission::Refused(Status::Forbidden);
        let admitted = Admission::Admitted("/".to_string());

        for hosts in [
            vec![format!("evil.example:{PORT}")],
            vec![format!("127.0.0.1:{}", PORT + 1)],
            vec!["127.0.0.1".to_string()],
            vec![],
            vec![format!("127.0.0.1:{PORT}"), format!("evil.example:{PORT}")],
        ] {
            assert_eq!(
                gate.admit_now(&with(hosts.clone(), vec![])),
                forbidden,
                "{hosts:?}"
            );
        }
        for origin in [
            "http://evil.example",
            "null",
            "https://127.0.0.1:4173",
            "http://127.0.0.1:4174",
            "http://localhost:4173.evil.example",
        ] {
            assert_eq!(
                gate.admit_now(&with(vec![format!("localhost:{PORT}")], vec![
                    origin.to_string()
                ])),
                forbidden,
                "{origin}"
            );
        }
        assert_eq!(
            gate.admit_now(&with(vec![format!("LocalHost:{PORT}")], vec![format!(
                "http://localhost:{PORT}"
            )])),
            admitted
        );
        assert_eq!(
            gate.admit_now(&with(vec![format!("127.0.0.1:{PORT}")], vec![format!(
                "http://127.0.0.1:{PORT}"
            )])),
            admitted
        );
    }

    /// Only GET is served, whatever path it names.
    #[test]
    fn only_get_is_served() {
        let gate = gate();

        for method in ["POST", "HEAD", "PUT", "get"] {
            let request = Request::builder()
                .method(method.to_string())
                .path(format!("/v/{}/", gate.token().secret()))
                .hosts(vec![format!("127.0.0.1:{PORT}")])
                .build();
            assert_eq!(
                gate.admit_now(&request),
                Admission::Refused(Status::MethodNotAllowed),
                "{method}"
            );
        }
    }

    /// The address carries the token as its path.
    #[test]
    fn the_url_carries_the_token_as_its_path() {
        let gate = gate();

        assert_eq!(
            gate.url(),
            format!("http://127.0.0.1:{PORT}/v/{}/", gate.token().secret())
        );
    }

    /// The opening address names a one-time code under `/open/`, never the
    /// token; that code redirects to the token's path once, and a second
    /// use, a wrong code, an expired code or a code with no mint is
    /// forbidden.
    #[test]
    fn a_one_time_code_redirects_once() {
        let gate = gate();
        let secret = gate.token().secret().to_string();
        let minted = Instant::now();
        let forbidden = Admission::Refused(Status::Forbidden);

        assert_eq!(gate.admit_now(&request("/open/anything")), forbidden);

        let url = gate.open_url(minted).unwrap();
        let path = url
            .strip_prefix(&format!("http://127.0.0.1:{PORT}"))
            .unwrap()
            .to_string();
        assert!(path.starts_with("/open/"), "{url}");
        assert!(!url.contains(&secret), "{url}");
        assert_eq!(
            gate.admit(&request(&path), minted + Duration::from_secs(1)),
            Admission::Redirect(format!("/v/{secret}/"))
        );
        assert_eq!(gate.admit(&request(&path), minted), forbidden);

        let path = gate
            .open_url(minted)
            .unwrap()
            .replace(&format!("http://127.0.0.1:{PORT}"), "");
        assert_eq!(
            gate.admit(&request(&path), minted + CODE_LIFETIME),
            forbidden
        );

        let path = gate
            .open_url(minted)
            .unwrap()
            .replace(&format!("http://127.0.0.1:{PORT}"), "");
        assert_eq!(
            gate.admit(&request(&format!("/open/{}", "0".repeat(64))), minted),
            forbidden
        );
        assert_eq!(gate.admit(&request(&path), minted), forbidden);
    }

    /// A one-time code is still held to the method, `Host` and `Origin`.
    #[test]
    fn a_one_time_code_is_checked_like_any_request() {
        let gate = gate();
        let path = gate
            .open_url(Instant::now())
            .unwrap()
            .replace(&format!("http://127.0.0.1:{PORT}"), "");

        let foreign = Request::builder()
            .method(GET.to_string())
            .path(path.clone())
            .hosts(vec![format!("evil.example:{PORT}")])
            .build();

        assert_eq!(
            gate.admit_now(&foreign),
            Admission::Refused(Status::Forbidden)
        );
        assert_eq!(
            gate.admit_now(&request(&path)),
            Admission::Redirect(format!("/v/{}/", gate.token().secret())),
            "a refused host never reaches the code"
        );
    }
}

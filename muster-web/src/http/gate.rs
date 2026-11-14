use getset::Getters;
use muster_domain::value::LocalPort;
use typed_builder::TypedBuilder;

use super::{admission::Admission, request::Request, status::Status, token::ViewToken};

/// The only method the view serves.
pub const GET: &str = "GET";
/// What every admitted path starts with, before the token.
const CAPABILITY_PREFIX: &str = "/v/";
/// What separates a path's segments.
const SEGMENT_SEPARATOR: char = '/';
/// The hosts a request may name, each followed by `:<port>`.
const LOOPBACK_HOSTS: [&str; 2] = ["127.0.0.1", "localhost"];
/// The scheme an allowed `Origin` names.
const ORIGIN_SCHEME: &str = "http://";

/// Decides which requests the view serves: GET only, naming the view's own
/// loopback address, from no foreign origin, under the path that carries its
/// token. No cookie is used: a loopback cookie would reach every port on the
/// machine, the dev servers muster runs among them.
#[derive(Clone, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Gate {
    /// The port the view listens on.
    port: LocalPort,
    /// The secret every admitted path carries.
    token: ViewToken,
}

impl Gate {
    /// Admits or refuses `request`, checking in order: the method, the
    /// `Host`, the `Origin`, then the token as the path's `/v/<token>/`
    /// prefix. An admitted request carries its path below that prefix.
    pub fn admit(&self, request: &Request) -> Admission {
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
        format!(
            "{ORIGIN_SCHEME}{}:{}{CAPABILITY_PREFIX}{}{SEGMENT_SEPARATOR}",
            LOOPBACK_HOSTS[0],
            self.port,
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
    use super::*;

    /// The port the gate under test listens on.
    const PORT: u16 = 4173;

    /// A gate on [`PORT`] with a fresh token.
    fn gate() -> Gate {
        Gate::builder()
            .port(LocalPort::try_new(PORT).unwrap())
            .token(ViewToken::generate().unwrap())
            .build()
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
            ("/api/log/shop/web", format!("/v/{secret}/api/log/shop/web")),
            ("/nope", format!("/v/{secret}/nope")),
        ] {
            assert_eq!(
                gate.admit(&request(&path)),
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
                gate.admit(&request(&path)),
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
                gate.admit(&with(hosts.clone(), vec![])),
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
                gate.admit(&with(vec![format!("localhost:{PORT}")], vec![
                    origin.to_string()
                ])),
                forbidden,
                "{origin}"
            );
        }
        assert_eq!(
            gate.admit(&with(vec![format!("LocalHost:{PORT}")], vec![format!(
                "http://localhost:{PORT}"
            )])),
            admitted
        );
        assert_eq!(
            gate.admit(&with(vec![format!("127.0.0.1:{PORT}")], vec![format!(
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
                gate.admit(&request),
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
}

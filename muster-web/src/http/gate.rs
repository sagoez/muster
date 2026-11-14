use getset::Getters;
use muster_domain::value::LocalPort;
use typed_builder::TypedBuilder;

use super::{admission::Admission, request::Request, status::Status, token::ViewToken};

/// The only method the view serves.
pub const GET: &str = "GET";
/// The path the first load carries the token on, and the redirect's target.
pub const SHELL_PATH: &str = "/";
/// The query parameter the first load carries the token in.
const KEY_PARAMETER: &str = "key";
/// The hosts a request may name, each followed by `:<port>`.
const LOOPBACK_HOSTS: [&str; 2] = ["127.0.0.1", "localhost"];
/// The scheme an allowed `Origin` names.
const ORIGIN_SCHEME: &str = "http://";
/// What every cookie name starts with, before the view's port.
const COOKIE_PREFIX: &str = "muster-view-";
/// The attributes the view's cookie is set with: out of scripts' reach and
/// never sent by another site's page.
const COOKIE_ATTRIBUTES: &str = "HttpOnly; SameSite=Strict; Path=/";
/// What separates the cookies of one `Cookie` header.
const COOKIE_SEPARATOR: char = ';';
/// What separates the parameters of a query.
const QUERY_SEPARATOR: char = '&';
/// What separates a name from its value, in a cookie or a query parameter.
const PAIR_SEPARATOR: char = '=';

/// Decides which requests the view serves: GET only, naming the view's own
/// loopback address, from no foreign origin, carrying its token.
#[derive(Clone, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Gate {
    /// The port the view listens on.
    port: LocalPort,
    /// The secret every admitted request carries.
    token: ViewToken,
}

impl Gate {
    /// Admits, redirects or refuses `request`, checking in order: the
    /// method, the `Host`, the `Origin`, the cookie, then the first load's
    /// key.
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
        if request
            .cookies()
            .iter()
            .any(|cookies| self.carries_cookie(cookies))
        {
            return Admission::Admitted;
        }
        if request.path() == SHELL_PATH
            && request
                .query()
                .as_deref()
                .is_some_and(|query| self.carries_key(query))
        {
            return Admission::GrantCookie;
        }
        Admission::Refused(Status::Forbidden)
    }

    /// The `Set-Cookie` value that admits a browser from now on.
    pub fn cookie(&self) -> String {
        format!(
            "{}{PAIR_SEPARATOR}{}; {COOKIE_ATTRIBUTES}",
            self.cookie_name(),
            self.token.secret()
        )
    }

    /// The address a browser opens first, carrying the token.
    pub fn first_url(&self) -> String {
        format!(
            "{ORIGIN_SCHEME}{}:{}{SHELL_PATH}?{KEY_PARAMETER}{PAIR_SEPARATOR}{}",
            LOOPBACK_HOSTS[0],
            self.port,
            self.token.secret()
        )
    }

    /// The cookie's name, which carries the port so two views, or one
    /// restarted on the same port, never read each other's cookie.
    fn cookie_name(&self) -> String {
        format!("{COOKIE_PREFIX}{}", self.port)
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

    /// Whether the `Cookie` header value `cookies` holds this view's cookie
    /// with its token.
    fn carries_cookie(&self, cookies: &str) -> bool {
        let name = self.cookie_name();
        cookies
            .split(COOKIE_SEPARATOR)
            .filter_map(|cookie| cookie.trim().split_once(PAIR_SEPARATOR))
            .any(|(key, value)| key == name && self.token.matches(value))
    }

    /// Whether `query` holds the token as its `key` parameter.
    fn carries_key(&self, query: &str) -> bool {
        query
            .split(QUERY_SEPARATOR)
            .filter_map(|parameter| parameter.split_once(PAIR_SEPARATOR))
            .any(|(key, value)| key == KEY_PARAMETER && self.token.matches(value))
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

    /// A GET for `path` naming `127.0.0.1:<PORT>`, with `query` and
    /// `cookies`.
    fn request(path: &str, query: Option<String>, cookies: Vec<String>) -> Request {
        Request::builder()
            .method(GET.to_string())
            .path(path.to_string())
            .query(query)
            .hosts(vec![format!("127.0.0.1:{PORT}")])
            .cookies(cookies)
            .build()
    }

    /// The view's cookie as a browser sends it back, among others.
    fn cookie_header(secret: &str) -> String {
        format!("theme=dark; muster-view-{PORT}={secret}; other=1")
    }

    /// No token is forbidden; the key on the first load grants the cookie;
    /// a wrong key is forbidden; the cookie admits every route.
    #[test]
    fn only_the_token_admits() {
        let gate = gate();
        let secret = gate.token().secret().to_string();

        assert_eq!(
            gate.admit(&request("/", None, vec![])),
            Admission::Refused(Status::Forbidden)
        );
        assert_eq!(
            gate.admit(&request("/", Some(format!("key={secret}")), vec![])),
            Admission::GrantCookie
        );
        assert_eq!(
            gate.admit(&request("/", Some(format!("a=1&key={secret}")), vec![])),
            Admission::GrantCookie
        );
        let wrong = ViewToken::generate().unwrap();
        assert_eq!(
            gate.admit(&request(
                "/",
                Some(format!("key={}", wrong.secret())),
                vec![]
            )),
            Admission::Refused(Status::Forbidden)
        );
        assert_eq!(
            gate.admit(&request("/events", Some(format!("key={secret}")), vec![])),
            Admission::Refused(Status::Forbidden),
            "the key is taken on the first load only"
        );
        for path in ["/", "/events", "/api/log/shop/web", "/assets/x", "/nope"] {
            assert_eq!(
                gate.admit(&request(path, None, vec![cookie_header(&secret)])),
                Admission::Admitted,
                "{path}"
            );
        }
        assert_eq!(
            gate.admit(&request("/", None, vec![cookie_header(wrong.secret())])),
            Admission::Refused(Status::Forbidden)
        );
    }

    /// The cookie names its port, so one set by a view on another port is
    /// refused.
    #[test]
    fn a_cookie_for_another_port_is_refused() {
        let gate = gate();
        let secret = gate.token().secret();

        let other_port = format!("muster-view-{}={secret}", PORT + 1);

        assert_eq!(
            gate.admit(&request("/", None, vec![other_port])),
            Admission::Refused(Status::Forbidden)
        );
    }

    /// A foreign host or port in `Host`, a missing or repeated `Host`, or a
    /// foreign `Origin` is refused; the loopback names with the view's port
    /// are admitted.
    #[test]
    fn a_foreign_host_or_origin_is_refused() {
        let gate = gate();
        let cookie = cookie_header(gate.token().secret());
        let with = |hosts: Vec<String>, origins: Vec<String>| {
            Request::builder()
                .method(GET.to_string())
                .path("/".to_string())
                .hosts(hosts)
                .origins(origins)
                .cookies(vec![cookie.clone()])
                .build()
        };
        let forbidden = Admission::Refused(Status::Forbidden);

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
            Admission::Admitted
        );
        assert_eq!(
            gate.admit(&with(vec![format!("127.0.0.1:{PORT}")], vec![format!(
                "http://127.0.0.1:{PORT}"
            )])),
            Admission::Admitted
        );
    }

    /// Only GET is served, whatever else the request carries.
    #[test]
    fn only_get_is_served() {
        let gate = gate();
        let cookie = cookie_header(gate.token().secret());

        for method in ["POST", "HEAD", "PUT", "get"] {
            let request = Request::builder()
                .method(method.to_string())
                .path("/".to_string())
                .hosts(vec![format!("127.0.0.1:{PORT}")])
                .cookies(vec![cookie.clone()])
                .build();
            assert_eq!(
                gate.admit(&request),
                Admission::Refused(Status::MethodNotAllowed),
                "{method}"
            );
        }
    }

    /// The cookie is the port's, carries the token, and is out of scripts'
    /// and other sites' reach; the first address carries the token.
    #[test]
    fn the_cookie_and_first_url_carry_the_token() {
        let gate = gate();
        let secret = gate.token().secret();

        assert_eq!(
            gate.cookie(),
            format!("muster-view-{PORT}={secret}; HttpOnly; SameSite=Strict; Path=/")
        );
        assert_eq!(
            gate.first_url(),
            format!("http://127.0.0.1:{PORT}/?key={secret}")
        );
    }
}

use getset::Getters;
use httparse::Header;
use typed_builder::TypedBuilder;

/// The `Host` header's name.
const HOST: &str = "host";
/// The `Origin` header's name.
const ORIGIN: &str = "origin";
/// The `Cookie` header's name.
const COOKIE: &str = "cookie";
/// What separates a request target's path from its query.
const QUERY_MARK: char = '?';

/// What the view reads of one request head: its method, target and the
/// headers admission checks. A header sent more than once keeps every value,
/// so none can hide behind another; one that is not UTF-8 reads with
/// replacement characters, which match nothing allowed.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Request {
    /// The method, as sent.
    method: String,
    /// The target's path, before any query.
    path: String,
    /// The target's query, without the `?`.
    #[builder(default)]
    query: Option<String>,
    /// Every `Host` value.
    #[builder(default)]
    hosts: Vec<String>,
    /// Every `Origin` value.
    #[builder(default)]
    origins: Vec<String>,
    /// Every `Cookie` value.
    #[builder(default)]
    cookies: Vec<String>,
}

impl Request {
    /// The request a complete head parsed into: its `method`, `target` and
    /// `headers`.
    pub fn from_head(method: &str, target: &str, headers: &[Header<'_>]) -> Self {
        let (path, query) = match target.split_once(QUERY_MARK) {
            Some((path, query)) => (path, Some(query.to_string())),
            None => (target, None),
        };
        let values = |name: &str| {
            headers
                .iter()
                .filter(|header| header.name.eq_ignore_ascii_case(name))
                .map(|header| String::from_utf8_lossy(header.value).into_owned())
                .collect::<Vec<_>>()
        };
        Self::builder()
            .method(method.to_string())
            .path(path.to_string())
            .query(query)
            .hosts(values(HOST))
            .origins(values(ORIGIN))
            .cookies(values(COOKIE))
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The target splits at its first `?`, and each checked header keeps
    /// every value whatever its case.
    #[test]
    fn a_head_keeps_its_path_query_and_every_checked_header() {
        let headers = [
            Header {
                name: "Host",
                value: b"127.0.0.1:4000",
            },
            Header {
                name: "HOST",
                value: b"evil.example",
            },
            Header {
                name: "Origin",
                value: b"\xffnot-utf8",
            },
            Header {
                name: "cookie",
                value: b"a=b",
            },
            Header {
                name: "Accept",
                value: b"*/*",
            },
        ];

        let request = Request::from_head("GET", "/?key=a?b", &headers);

        assert_eq!(request.path(), "/");
        assert_eq!(request.query().as_deref(), Some("key=a?b"));
        assert_eq!(request.hosts(), &["127.0.0.1:4000", "evil.example"]);
        assert_eq!(request.origins(), &["\u{fffd}not-utf8"]);
        assert_eq!(request.cookies(), &["a=b"]);
    }
}

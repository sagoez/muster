use getset::Getters;
use httparse::Header;
use typed_builder::TypedBuilder;

/// The `Host` header's name.
const HOST: &str = "host";
/// The `Origin` header's name.
const ORIGIN: &str = "origin";
/// What separates a request target's path from its query.
const QUERY_MARK: char = '?';

/// What the view reads of one request head: its method, its path and the
/// headers admission checks. The query is dropped, since no route reads
/// one, and so are cookies, which admit nothing. A header sent more than
/// once keeps every value, so none can hide behind another; one that is not
/// UTF-8 reads with replacement characters, which match nothing allowed.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Request {
    /// The method, as sent.
    method: String,
    /// The target's path, before any query.
    path: String,
    /// Every `Host` value.
    #[builder(default)]
    hosts: Vec<String>,
    /// Every `Origin` value.
    #[builder(default)]
    origins: Vec<String>,
}

impl Request {
    /// The request a complete head parsed into: its `method`, `target` and
    /// `headers`.
    pub fn from_head(method: &str, target: &str, headers: &[Header<'_>]) -> Self {
        let path = target
            .split_once(QUERY_MARK)
            .map_or(target, |(path, _)| path);
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
            .hosts(values(HOST))
            .origins(values(ORIGIN))
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The target loses its query at the first `?`, and each checked header
    /// keeps every value whatever its case.
    #[test]
    fn a_head_keeps_its_path_and_every_checked_header() {
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
                name: "Accept",
                value: b"*/*",
            },
        ];

        let request = Request::from_head("GET", "/v/abc/?key=a?b", &headers);

        assert_eq!(request.path(), "/v/abc/");
        assert_eq!(request.hosts(), &["127.0.0.1:4000", "evil.example"]);
        assert_eq!(request.origins(), &["\u{fffd}not-utf8"]);
    }
}

use percent_encoding::percent_decode_str;

use super::{asset::ASSET_ROOT, log::LogTarget};

/// The shell's path below the token's.
const SHELL_PATH: &str = "/";
/// The path of the live stream.
const EVENTS_PATH: &str = "/events";
/// The path of the log route, which names its process in its query.
const LOG_PATH: &str = "/api/log";
/// The query key naming a log's project.
const PROJECT_KEY: &str = "project";
/// The query key naming a log's process.
const NAME_KEY: &str = "name";
/// What separates a query's pairs.
const PAIR_SEPARATOR: char = '&';
/// What separates a pair's key from its value.
const VALUE_MARK: char = '=';
/// What separates a path's segments.
const SEGMENT_SEPARATOR: char = '/';

/// What an admitted request's path names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// The page's shell.
    Shell,
    /// One embedded asset of this version, by its path under the asset root.
    Asset(String),
    /// The live stream.
    Events,
    /// One process's log tail.
    Log(LogTarget),
    /// Nothing the view serves.
    NotFound,
}

impl Route {
    /// The route a `path` below the token's and its `query` name. An asset
    /// under another version's root names nothing, and so does a log
    /// request whose query does not hold exactly one non-empty `project` and
    /// one non-empty `name`, and nothing else, each decoding to UTF-8. The
    /// names travel in the query so a name such as `..` reaches the floor
    /// as itself, never folded into the path.
    pub fn parse(path: &str, query: Option<&str>) -> Self {
        if path == SHELL_PATH {
            return Self::Shell;
        }
        if path == EVENTS_PATH {
            return Self::Events;
        }
        if let Some(asset) = path
            .strip_prefix(SEGMENT_SEPARATOR)
            .and_then(|rest| rest.strip_prefix(ASSET_ROOT))
            .and_then(|rest| rest.strip_prefix(SEGMENT_SEPARATOR))
            .filter(|asset| !asset.is_empty())
        {
            return Self::Asset(asset.to_string());
        }
        if path == LOG_PATH {
            return query.and_then(log_target).map_or(Self::NotFound, Self::Log);
        }
        Self::NotFound
    }
}

/// The process `query` names as `project=<p>&name=<n>`, in either order,
/// each value decoded once; none when a key is missing, repeated or
/// unknown.
fn log_target(query: &str) -> Option<LogTarget> {
    let (mut project, mut name) = (None, None);
    for pair in query.split(PAIR_SEPARATOR) {
        let (key, value) = pair.split_once(VALUE_MARK)?;
        let slot = match key {
            PROJECT_KEY => &mut project,
            NAME_KEY => &mut name,
            _ => return None,
        };
        if slot.replace(decoded(value)?).is_some() {
            return None;
        }
    }
    Some(LogTarget::builder().project(project?).name(name?).build())
}

/// `value` percent-decoded once, when it is non-empty and decodes to UTF-8.
/// A `+` stays a `+`: the page encodes a space as `%20`.
fn decoded(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    percent_decode_str(value)
        .decode_utf8()
        .ok()
        .map(|text| text.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The log route for `project` and `name`.
    fn log(project: &str, name: &str) -> Route {
        Route::Log(
            LogTarget::builder()
                .project(project.to_string())
                .name(name.to_string())
                .build(),
        )
    }

    /// The log route at `query`.
    fn logged(query: &str) -> Route {
        Route::parse(LOG_PATH, Some(query))
    }

    /// Each log value is decoded exactly once, in either order, and a dot
    /// name stays itself; a missing, repeated, unknown or empty key names
    /// nothing, and so does a value that is not UTF-8.
    #[test]
    fn a_log_route_decodes_each_value_once() {
        assert_eq!(
            logged("project=shop&name=dev%20server"),
            log("shop", "dev server")
        );
        assert_eq!(logged("name=web&project=my%20shop"), log("my shop", "web"));
        assert_eq!(logged("project=shop&name=a%252Fb"), log("shop", "a%2Fb"));
        assert_eq!(logged("project=shop&name=a%2Fb"), log("shop", "a/b"));
        assert_eq!(logged("project=..&name=."), log("..", "."));
        assert_eq!(logged("project=shop&name=a%26b%3Dc"), log("shop", "a&b=c"));
        assert_eq!(logged("project=shop&name=a+b"), log("shop", "a+b"));
        for query in [
            "project=shop",
            "name=web",
            "project=shop&name=web&name=api",
            "project=shop&name=web&extra=1",
            "project=shop&name=",
            "project=&name=web",
            "project=shop&name",
            "project=shop&name=%FF",
            "",
        ] {
            assert_eq!(logged(query), Route::NotFound, "{query}");
        }
        assert_eq!(Route::parse(LOG_PATH, None), Route::NotFound);
        assert_eq!(Route::parse("/api/log/shop/web", None), Route::NotFound);
    }

    /// Assets are served under this version's root only.
    #[test]
    fn asset_urls_carry_the_version() {
        let version = env!("CARGO_PKG_VERSION");

        assert_eq!(
            Route::parse(&format!("/assets/v{version}/js/app.js"), None),
            Route::Asset("js/app.js".to_string())
        );
        for path in [
            "/assets/v0.0.0-other/js/app.js".to_string(),
            "/assets/js/app.js".to_string(),
            format!("/assets/v{version}/"),
            format!("/assets/v{version}x/js/app.js"),
        ] {
            assert_eq!(Route::parse(&path, None), Route::NotFound, "{path}");
        }
    }

    /// The shell and the stream have fixed paths; anything else names
    /// nothing.
    #[test]
    fn fixed_routes_and_unknown_paths() {
        assert_eq!(Route::parse("/", None), Route::Shell);
        assert_eq!(Route::parse("/events", None), Route::Events);
        for path in ["/index.html", "/events/", "", "/api/log/"] {
            assert_eq!(Route::parse(path, None), Route::NotFound, "{path}");
        }
    }
}

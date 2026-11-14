use percent_encoding::percent_decode_str;

use super::{asset::ASSET_ROOT, log_target::LogTarget};
use crate::http::SHELL_PATH;

/// The path of the live stream.
const EVENTS_PATH: &str = "/events";
/// What a log route's path starts with, before `<project>/<name>`.
const LOG_PREFIX: &str = "/api/log/";
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
    /// The route `path` names. An asset under another version's root, a log
    /// path without exactly two non-empty segments, or a segment that does
    /// not decode to UTF-8 names nothing.
    pub fn parse(path: &str) -> Self {
        if path == SHELL_PATH {
            return Self::Shell;
        }
        if path == EVENTS_PATH {
            return Self::Events;
        }
        if let Some(asset) = path
            .strip_prefix(ASSET_ROOT)
            .and_then(|rest| rest.strip_prefix(SEGMENT_SEPARATOR))
            .filter(|asset| !asset.is_empty())
        {
            return Self::Asset(asset.to_string());
        }
        path.strip_prefix(LOG_PREFIX)
            .and_then(log_target)
            .map_or(Self::NotFound, Self::Log)
    }
}

/// The process `segments` names as `<project>/<name>`, each decoded once.
fn log_target(segments: &str) -> Option<LogTarget> {
    let (project, name) = segments.split_once(SEGMENT_SEPARATOR)?;
    if name.contains(SEGMENT_SEPARATOR) {
        return None;
    }
    Some(
        LogTarget::builder()
            .project(decoded(project)?)
            .name(decoded(name)?)
            .build(),
    )
}

/// `segment` percent-decoded once, when it is non-empty and decodes to
/// UTF-8.
fn decoded(segment: &str) -> Option<String> {
    if segment.is_empty() {
        return None;
    }
    percent_decode_str(segment)
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

    /// Each log segment is decoded exactly once, and a third segment names
    /// nothing.
    #[test]
    fn a_log_route_decodes_each_segment_once() {
        assert_eq!(
            Route::parse("/api/log/shop/dev%20server"),
            log("shop", "dev server")
        );
        assert_eq!(Route::parse("/api/log/shop/a%252Fb"), log("shop", "a%2Fb"));
        assert_eq!(Route::parse("/api/log/shop/a%2Fb"), log("shop", "a/b"));
        assert_eq!(
            Route::parse("/api/log/my%20shop/web"),
            log("my shop", "web")
        );
        for path in [
            "/api/log/shop/a/b",
            "/api/log/shop",
            "/api/log/shop/",
            "/api/log//web",
            "/api/log/shop/%FF",
        ] {
            assert_eq!(Route::parse(path), Route::NotFound, "{path}");
        }
    }

    /// Assets are served under this version's root only.
    #[test]
    fn asset_urls_carry_the_version() {
        let version = env!("CARGO_PKG_VERSION");

        assert_eq!(
            Route::parse(&format!("/assets/v{version}/js/app.js")),
            Route::Asset("js/app.js".to_string())
        );
        for path in [
            "/assets/v0.0.0-other/js/app.js".to_string(),
            "/assets/js/app.js".to_string(),
            format!("/assets/v{version}/"),
            format!("/assets/v{version}x/js/app.js"),
        ] {
            assert_eq!(Route::parse(&path), Route::NotFound, "{path}");
        }
    }

    /// The shell and the stream have fixed paths; anything else names
    /// nothing.
    #[test]
    fn fixed_routes_and_unknown_paths() {
        assert_eq!(Route::parse("/"), Route::Shell);
        assert_eq!(Route::parse("/events"), Route::Events);
        for path in ["/index.html", "/events/", "", "/api/log"] {
            assert_eq!(Route::parse(path), Route::NotFound, "{path}");
        }
    }
}

use std::{cmp::Ordering, path::PathBuf};

use getset::Getters;
use semver::Version;
use typed_builder::TypedBuilder;

/// A service that answered a client's hello: its version, and the binary it
/// runs when it is new enough to say.
#[derive(Debug, Clone, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct RunningService {
    /// Its muster version, or none when it is too old to answer a hello.
    version: Option<String>,
    /// The muster binary it runs, when it says.
    binary: Option<PathBuf>,
}

impl RunningService {
    /// How a client of version `ours` compares with this service: `Greater`
    /// when the client is newer and replaces it, `Less` when the service is
    /// newer. A version that does not parse, or a service too old to say, is
    /// older than any client.
    pub fn compared_with(&self, ours: &Version) -> Ordering {
        match self
            .version
            .as_deref()
            .and_then(|theirs| Version::parse(theirs).ok())
        {
            Some(theirs) => ours.cmp(&theirs),
            None => Ordering::Greater,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A service of `version`.
    fn service(version: Option<&str>) -> RunningService {
        RunningService::builder()
            .version(version.map(str::to_string))
            .binary(None)
            .build()
    }

    #[test]
    fn only_a_newer_client_replaces_the_service() {
        let ours = Version::parse("0.9.1").unwrap();

        assert_eq!(
            service(Some("0.9.0")).compared_with(&ours),
            Ordering::Greater
        );
        assert_eq!(service(Some("0.9.1")).compared_with(&ours), Ordering::Equal);
        assert_eq!(service(Some("0.10.0")).compared_with(&ours), Ordering::Less);
        assert_eq!(
            service(Some("0.9.1-rc.1")).compared_with(&ours),
            Ordering::Greater,
            "a pre-release is older than its release"
        );
        assert_eq!(service(None).compared_with(&ours), Ordering::Greater);
        assert_eq!(
            service(Some("garbage")).compared_with(&ours),
            Ordering::Greater
        );
    }
}

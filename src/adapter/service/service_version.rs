use nutype::nutype;
use semver::Version;

use crate::constants::APP_VERSION;

/// A muster build's semantic version, as a client and the service exchange
/// it in their hello, so the newer one can replace the older.
#[nutype(derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Display,
    FromStr,
    Serialize,
    Deserialize
))]
pub struct ServiceVersion(Version);

impl ServiceVersion {
    /// This build's version. Cargo only builds a package with a semantic
    /// version, so the fallback to `0.0.0` is never taken.
    pub fn current() -> Self {
        Self::new(Version::parse(APP_VERSION).unwrap_or_else(|_| Version::new(0, 0, 0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A version travels as its plain text and refuses anything else.
    #[test]
    fn a_version_travels_as_its_text_and_refuses_anything_else() {
        let version: ServiceVersion = serde_json::from_str("\"0.9.1-rc.1\"").unwrap();

        assert_eq!(serde_json::to_string(&version).unwrap(), "\"0.9.1-rc.1\"");
        assert!(serde_json::from_str::<ServiceVersion>("\"nine\"").is_err());
        assert_eq!(ServiceVersion::current().to_string(), APP_VERSION);
    }

    /// A pre-release orders before its release.
    #[test]
    fn a_pre_release_is_older_than_its_release() {
        let release: ServiceVersion = "0.9.1".parse().unwrap();
        let candidate: ServiceVersion = "0.9.1-rc.1".parse().unwrap();

        assert!(candidate < release);
    }
}

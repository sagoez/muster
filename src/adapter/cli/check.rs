use std::path::PathBuf;

use crate::{adapter::config::YamlConfigSource, domain::port::ConfigSource};

/// Separator between links of a reported error chain.
const CHAIN_SEPARATOR: &str = ": ";

/// Result of validating a workspace config.
pub enum CheckOutcome {
    /// The config loaded and validated without errors.
    Valid,
    /// The config failed to load; the report is the full error chain.
    Invalid(String),
}

/// Validates the workspace config at `config_path`.
pub fn check(config_path: PathBuf) -> CheckOutcome {
    let source = YamlConfigSource::builder().path(config_path).build();
    match source.load() {
        Ok(_) => CheckOutcome::Valid,
        Err(error) => CheckOutcome::Invalid(error_chain(&error)),
    }
}

/// Formats an error and its sources as one line, outermost first, separated by
/// `": "`. Reused by `muster doctor` to report multiple findings uniformly.
pub fn error_chain(error: &dyn std::error::Error) -> String {
    let mut report = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        report.push_str(CHAIN_SEPARATOR);
        report.push_str(&cause.to_string());
        source = cause.source();
    }
    report
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::*;

    fn temp_config(tag: &str, content: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("muster-check-{tag}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("muster.yml");
        fs::write(&path, content).unwrap();
        path
    }

    /// A valid workspace reports Valid.
    #[test]
    fn a_valid_config_passes() {
        let path = temp_config("ok", "agents: []\nterminals: []\ncommands: []\n");
        assert!(matches!(check(path), CheckOutcome::Valid));
    }

    /// Broken YAML reports the parse failure.
    #[test]
    fn broken_yaml_reports_the_error() {
        let path = temp_config("bad", "agents: [unclosed\n");
        match check(path) {
            CheckOutcome::Invalid(report) => assert!(!report.is_empty()),
            CheckOutcome::Valid => panic!("must fail"),
        }
    }

    /// A missing file reports rather than panics.
    #[test]
    fn a_missing_file_reports() {
        let path = std::env::temp_dir().join("muster-check-missing/muster.yml");
        assert!(matches!(check(path), CheckOutcome::Invalid(_)));
    }
}

use std::path::PathBuf;

use super::error::CliError;
use crate::{
    adapter::config::YamlConfigSource,
    domain::{config::ConfigError, port::ConfigSource},
};

/// Separator between links of a reported error chain.
const CHAIN_SEPARATOR: &str = ": ";
/// Suffix shared between `muster check` output and the doctor config probe.
pub const VALID_SUFFIX: &str = "is valid";

/// Result of validating a workspace config.
#[derive(Debug)]
pub enum CheckOutcome {
    /// The config loaded and validated without errors.
    Valid,
    /// The config failed to load; the report is the full error chain.
    Invalid(String),
}

/// Validates the workspace config at `config_path`. A config that fails to
/// parse or validate is a finding; a file that cannot be read at all is an
/// operational failure of the command itself.
///
/// # Errors
/// Returns [`CliError`] when the config cannot be read.
pub fn check(config_path: PathBuf) -> Result<CheckOutcome, CliError> {
    let source = YamlConfigSource::builder().path(config_path).build();
    match source.load() {
        Ok(_) => Ok(CheckOutcome::Valid),
        Err(error @ ConfigError::Read { .. }) => Err(CliError::Config(error)),
        Err(error) => Ok(CheckOutcome::Invalid(error_chain(&error))),
    }
}

/// Formats an error and its sources as one line, outermost first, separated by
/// `": "`. Reused by `muster doctor` to report multiple findings uniformly.
pub fn error_chain(error: &dyn std::error::Error) -> String {
    let mut report = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        // Many error displays already embed their source text; appending
        // those again would repeat every diagnostic verbatim.
        if !report.contains(&text) {
            report.push_str(CHAIN_SEPARATOR);
            report.push_str(&text);
        }
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
        assert!(matches!(check(path), Ok(CheckOutcome::Valid)));
    }

    /// Broken YAML reports the parse failure.
    #[test]
    fn broken_yaml_reports_the_error() {
        let path = temp_config("bad", "agents: [unclosed\n");
        match check(path) {
            Ok(CheckOutcome::Invalid(report)) => assert!(!report.is_empty()),
            other => panic!("must be a finding: {other:?}"),
        }
    }

    /// A missing file is an operational failure, not a validation finding.
    #[test]
    fn a_missing_file_is_a_command_failure() {
        let path = std::env::temp_dir().join("muster-check-missing/muster.yml");
        assert!(matches!(check(path), Err(CliError::Config(_))));
    }

    /// An error whose display already embeds its source text.
    #[derive(Debug)]
    struct EmbeddingError(WrappedError);

    impl std::fmt::Display for EmbeddingError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "could not parse: {}", self.0)
        }
    }

    impl std::error::Error for EmbeddingError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    /// An error whose display is context-only, not embedding its source.
    #[derive(Debug)]
    struct ContextError(WrappedError);

    impl std::fmt::Display for ContextError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "loading the config failed")
        }
    }

    impl std::error::Error for ContextError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    #[derive(Debug)]
    struct WrappedError;

    impl std::fmt::Display for WrappedError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "unexpected token at line 3")
        }
    }

    impl std::error::Error for WrappedError {}

    /// Sources already rendered by the outer display are not appended again;
    /// sources adding new text still are.
    #[test]
    fn error_chain_skips_already_rendered_sources() {
        assert_eq!(
            error_chain(&EmbeddingError(WrappedError)),
            "could not parse: unexpected token at line 3"
        );
        assert_eq!(
            error_chain(&ContextError(WrappedError)),
            "loading the config failed: unexpected token at line 3"
        );
    }
}

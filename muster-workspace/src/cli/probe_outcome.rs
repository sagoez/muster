use serde::Serialize;

/// Severity of a probe result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProbeOutcome {
    /// Healthy.
    Ok,
    /// Advisory; does not fail the doctor run.
    Warn,
    /// Broken; the doctor run exits non-zero.
    Fail,
}

/// Severity of a probe result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// Healthy.
    Ok,
    /// Advisory; does not fail the doctor run.
    Warn,
    /// Broken; the doctor run exits non-zero.
    Fail,
}

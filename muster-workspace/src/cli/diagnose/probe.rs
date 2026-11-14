use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{doctor_check::DoctorCheck, probe_outcome::ProbeOutcome};

/// One diagnostic line: what was probed, how it went, and the detail text.
#[derive(Debug, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct Probe {
    /// What was probed.
    #[getset(get_copy = "pub")]
    check: DoctorCheck,
    /// Severity of the result.
    #[getset(get_copy = "pub")]
    outcome: ProbeOutcome,
    /// Human detail for the line.
    #[getset(get = "pub")]
    detail: String,
}

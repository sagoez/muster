use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::probe::Probe;

/// What `muster doctor --json` prints: every probe, and whether one failed.
#[derive(Debug, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct DoctorView<'a> {
    /// The probes, in the order they ran.
    #[getset(get = "pub")]
    checks: &'a [Probe],
    /// Whether any failed, which makes the command exit non-zero.
    #[getset(get_copy = "pub")]
    failed: bool,
}

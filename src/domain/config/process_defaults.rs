use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::process::{RestartPolicy, StopPolicy};

/// The values a new process takes for each field its definition leaves out,
/// read from the `default_*` settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ProcessDefaults {
    /// Whether `muster up` starts the process.
    autostart: bool,
    /// What happens when its child exits.
    restart: RestartPolicy,
    /// How it is stopped.
    stop: StopPolicy,
}

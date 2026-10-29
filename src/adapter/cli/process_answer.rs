use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::domain::{config::ProcessSpec, process::ProcessKind};

/// One process as `muster init` was told about it, before it joins the
/// section of its kind.
#[derive(Debug, Clone, Getters, CopyGetters, TypedBuilder)]
pub(super) struct ProcessAnswer {
    /// The section it joins.
    #[getset(get_copy = "pub(super)")]
    kind: ProcessKind,
    /// The process as it is written.
    #[getset(get = "pub(super)")]
    spec: ProcessSpec,
}

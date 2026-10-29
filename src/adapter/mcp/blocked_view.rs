use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::adapter::service::PortBlock;

/// Opens the hint on a blocked start, before the holder's project.
const HINT_OPEN: &str = "stop it with process_stop {project: '";
/// Between the holder's project and its name in the hint.
const HINT_NAME: &str = "', name: '";
/// Closes the hint.
const HINT_CLOSE: &str = "'}";

/// Why a process was not started, as `stack_status` shows it: the port it
/// needs, who holds it, and the call that frees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct BlockedView {
    /// The port its readiness check names.
    #[getset(get_copy = "pub")]
    port: u16,
    /// The process listening on it.
    #[getset(get = "pub")]
    holder: String,
    /// What frees it.
    #[getset(get = "pub")]
    hint: String,
}

impl From<&PortBlock> for BlockedView {
    fn from(block: &PortBlock) -> Self {
        let holder = block.holder();
        Self {
            port: block.port().into_inner(),
            holder: holder.to_string(),
            hint: format!(
                "{HINT_OPEN}{}{HINT_NAME}{}{HINT_CLOSE}",
                holder.project(),
                holder.process()
            ),
        }
    }
}

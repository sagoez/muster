use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::adapter::service::{PortBlock, PortOwner};

/// Opens the hint on a blocked start, before the holder's project.
const HINT_OPEN: &str = "stop it with process_stop {project: '";
/// Between the holder's project and its name in the hint.
const HINT_NAME: &str = "', name: '";
/// Closes the hint.
const HINT_CLOSE: &str = "'}";

/// Why a process was not started, as `stack_status` shows it: the port it
/// needs, who holds it, and the call that frees it when muster can.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct BlockedView {
    /// The port its readiness check names.
    #[getset(get_copy = "pub")]
    port: u16,
    /// The process listening on it.
    #[getset(get = "pub")]
    holder: String,
    /// What frees it, when the holder is a process muster supervises.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get = "pub")]
    hint: Option<String>,
}

impl From<&PortBlock> for BlockedView {
    fn from(block: &PortBlock) -> Self {
        let hint = match block.holder() {
            PortOwner::Supervised(holder) => Some(format!(
                "{HINT_OPEN}{}{HINT_NAME}{}{HINT_CLOSE}",
                holder.project(),
                holder.process()
            )),
            PortOwner::Other(_) => None,
        };
        Self {
            port: block.port().into_inner(),
            holder: block.holder().to_string(),
            hint,
        }
    }
}

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
    /// The first port it needs that was held.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        port::{OCCUPANT_COMMAND_CUT, OCCUPANT_COMMAND_MAX_CHARS, PortOccupant},
        value::{LocalPort, Pid},
    };

    /// A program holding the port with a huge command line is named by it
    /// cut to the cap and marked, never the whole argv.
    #[test]
    fn a_huge_holder_command_line_is_cut() {
        let block = PortBlock::builder()
            .port(LocalPort::try_new(5173).unwrap())
            .holder(PortOwner::Other(
                PortOccupant::builder()
                    .pid(Pid::try_new(4242).unwrap())
                    .command("z".repeat(OCCUPANT_COMMAND_MAX_CHARS * 1000))
                    .build(),
            ))
            .build();

        let view = BlockedView::from(&block);

        let shown = format!(
            "{}{OCCUPANT_COMMAND_CUT}",
            "z".repeat(OCCUPANT_COMMAND_MAX_CHARS)
        );
        assert!(view.holder().contains(&shown), "{}", view.holder());
        assert!(
            view.holder().chars().count() < OCCUPANT_COMMAND_MAX_CHARS * 2,
            "{}",
            view.holder()
        );
    }
}

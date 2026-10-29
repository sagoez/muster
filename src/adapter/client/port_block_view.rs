use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::port_owner_view::PortOwnerView;
use crate::adapter::service::PortBlock;

/// Why a process was not started, as `--json` shows it: the port it needs
/// and who holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct PortBlockView {
    /// The first of its ports found held.
    #[getset(get_copy = "pub")]
    port: u16,
    /// Who listens on it.
    #[getset(get = "pub")]
    holder: PortOwnerView,
}

impl From<&PortBlock> for PortBlockView {
    fn from(block: &PortBlock) -> Self {
        Self {
            port: block.port().into_inner(),
            holder: PortOwnerView::from(block.holder()),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        adapter::service::{PortHolder, PortOwner},
        domain::{
            port::PortOccupant,
            value::{LocalPort, Pid, ProcessName, ProjectName},
        },
    };

    /// Each kind of holder is tagged by `kind`, its fields in snake case.
    #[test]
    fn each_holder_is_tagged_by_its_kind() {
        let port = LocalPort::try_new(5173).unwrap();
        let supervised = PortBlock::builder()
            .port(port)
            .holder(PortOwner::Supervised(
                PortHolder::builder()
                    .project(ProjectName::try_new("admin").unwrap())
                    .process(ProcessName::try_new("web").unwrap())
                    .build(),
            ))
            .build();
        let other = PortBlock::builder()
            .port(port)
            .holder(PortOwner::Other(
                PortOccupant::builder()
                    .pid(Pid::try_new(4242).unwrap())
                    .command("python3 -m http.server 5173".to_string())
                    .build(),
            ))
            .build();

        assert_eq!(
            serde_json::to_value(PortBlockView::from(&supervised)).unwrap(),
            json!({"port": 5173, "holder": {"kind": "supervised", "project": "admin", "process": "web"}})
        );
        assert_eq!(
            serde_json::to_value(PortBlockView::from(&other)).unwrap(),
            json!({"port": 5173, "holder": {"kind": "other", "pid": 4242, "command": "python3 -m http.server 5173"}})
        );
    }
}

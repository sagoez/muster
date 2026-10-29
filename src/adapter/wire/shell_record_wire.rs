use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::WireForm;
use crate::domain::{
    occupancy::{ReportSeq, ShellId, ShellRecord},
    value::{Pid, ProjectRoot, StartToken},
};

/// A shell's last report, as its record file holds it.
#[derive(
    Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ShellRecordWire {
    /// The shell's process id.
    #[getset(get_copy = "pub")]
    pid: Pid,
    /// That process's start token.
    #[getset(get_copy = "pub")]
    start: StartToken,
    /// The prompt the report was made at.
    #[getset(get_copy = "pub")]
    seq: ReportSeq,
    /// The project its folder was in, or `null` outside every project.
    #[getset(get = "pub")]
    project: Option<ProjectRoot>,
}

impl WireForm for ShellRecord {
    type Wire = ShellRecordWire;

    fn to_wire(&self) -> ShellRecordWire {
        ShellRecordWire {
            pid: self.shell().pid(),
            start: self.shell().start(),
            seq: *self.seq(),
            project: self.project().clone(),
        }
    }

    fn from_wire(wire: ShellRecordWire) -> Result<Self, String> {
        Ok(Self::builder()
            .shell(ShellId::builder().pid(wire.pid).start(wire.start).build())
            .seq(wire.seq)
            .project(wire.project)
            .build())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A record is one flat object, its project `null` outside every
    /// project, and refuses a key it does not know.
    #[test]
    fn a_record_travels_flat_and_refuses_unknown_keys() {
        let record = |project: Option<&str>| {
            ShellRecord::builder()
                .shell(
                    ShellId::builder()
                        .pid(Pid::try_new(4242).unwrap())
                        .start(StartToken::try_new(77).unwrap())
                        .build(),
                )
                .seq(ReportSeq::new(3))
                .project(project.map(|root| ProjectRoot::try_new(PathBuf::from(root)).unwrap()))
                .build()
        };
        for (project, json) in [
            (
                Some("/home/me/shop"),
                r#"{"pid":4242,"start":77,"seq":3,"project":"/home/me/shop"}"#,
            ),
            (None, r#"{"pid":4242,"start":77,"seq":3,"project":null}"#),
        ] {
            assert_eq!(
                serde_json::to_string(&record(project).to_wire()).unwrap(),
                json
            );
            let read: ShellRecordWire = serde_json::from_str(json).unwrap();
            assert_eq!(ShellRecord::from_wire(read), Ok(record(project)));
        }
        assert!(
            serde_json::from_str::<ShellRecordWire>(
                r#"{"pid":4242,"start":77,"seq":3,"project":null,"cwd":"/"}"#
            )
            .is_err()
        );
    }
}

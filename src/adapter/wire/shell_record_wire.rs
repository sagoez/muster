use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::WireForm;
use crate::domain::{
    occupancy::{LoadId, ReportSeq, ReportStamp, ShellId, ShellRecord},
    value::{Pid, ProjectRoot, StartToken},
};

/// A shell's report, as its record file and the service protocol hold it.
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
    /// The hook load the report was made from.
    #[getset(get_copy = "pub")]
    load: LoadId,
    /// The prompt the report was made at, within that load.
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
            load: self.stamp().load(),
            seq: self.stamp().seq(),
            project: self.project().clone(),
        }
    }

    fn from_wire(wire: ShellRecordWire) -> Result<Self, String> {
        Ok(Self::builder()
            .shell(ShellId::builder().pid(wire.pid).start(wire.start).build())
            .stamp(ReportStamp::builder().load(wire.load).seq(wire.seq).build())
            .project(wire.project)
            .build())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::domain::occupancy::testing::stamp;

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
                .stamp(stamp(9, 3))
                .project(project.map(|root| ProjectRoot::try_new(PathBuf::from(root)).unwrap()))
                .build()
        };
        for (project, json) in [
            (
                Some("/home/me/shop"),
                r#"{"pid":4242,"start":77,"load":9,"seq":3,"project":"/home/me/shop"}"#,
            ),
            (
                None,
                r#"{"pid":4242,"start":77,"load":9,"seq":3,"project":null}"#,
            ),
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
                r#"{"pid":4242,"start":77,"load":9,"seq":3,"project":null,"cwd":"/"}"#
            )
            .is_err()
        );
    }
}

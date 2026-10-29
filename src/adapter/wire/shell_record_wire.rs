use std::path::PathBuf;

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
    /// The folder the shell was in.
    #[getset(get = "pub")]
    folder: PathBuf,
    /// The project its folder was in, or `null` outside every project.
    #[getset(get = "pub")]
    project: Option<ProjectRoot>,
    /// Projects it still counts in, around one registered about it.
    #[getset(get = "pub")]
    held: Vec<ProjectRoot>,
}

impl WireForm for ShellRecord {
    type Wire = ShellRecordWire;

    fn to_wire(&self) -> ShellRecordWire {
        ShellRecordWire {
            pid: self.shell().pid(),
            start: self.shell().start(),
            load: self.stamp().load(),
            seq: self.stamp().seq(),
            folder: self.folder().clone(),
            project: self.project().clone(),
            held: self.held().clone(),
        }
    }

    fn from_wire(wire: ShellRecordWire) -> Result<Self, String> {
        Ok(Self::builder()
            .shell(ShellId::builder().pid(wire.pid).start(wire.start).build())
            .stamp(ReportStamp::builder().load(wire.load).seq(wire.seq).build())
            .folder(wire.folder)
            .project(wire.project)
            .held(wire.held)
            .build())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::occupancy::testing::stamp;

    /// A record is one flat object, its project `null` outside every
    /// project, and refuses a key it does not know.
    #[test]
    fn a_record_travels_flat_and_refuses_unknown_keys() {
        let root = |path: &str| ProjectRoot::try_new(PathBuf::from(path)).unwrap();
        let record = |project: Option<&str>, held: &[&str]| {
            ShellRecord::builder()
                .shell(
                    ShellId::builder()
                        .pid(Pid::try_new(4242).unwrap())
                        .start(StartToken::try_new(77).unwrap())
                        .build(),
                )
                .stamp(stamp(9, 3))
                .folder(PathBuf::from("/home/me/shop/admin"))
                .project(project.map(root))
                .held(held.iter().map(|path| root(path)).collect())
                .build()
        };
        for (project, held, json) in [
            (
                Some("/home/me/shop/admin"),
                &["/home/me/shop"][..],
                r#"{"pid":4242,"start":77,"load":9,"seq":3,"folder":"/home/me/shop/admin","project":"/home/me/shop/admin","held":["/home/me/shop"]}"#,
            ),
            (
                None,
                &[][..],
                r#"{"pid":4242,"start":77,"load":9,"seq":3,"folder":"/home/me/shop/admin","project":null,"held":[]}"#,
            ),
        ] {
            assert_eq!(
                serde_json::to_string(&record(project, held).to_wire()).unwrap(),
                json
            );
            let read: ShellRecordWire = serde_json::from_str(json).unwrap();
            assert_eq!(ShellRecord::from_wire(read), Ok(record(project, held)));
        }
        assert!(
            serde_json::from_str::<ShellRecordWire>(
                r#"{"pid":4242,"start":77,"load":9,"seq":3,"folder":"/","project":null,"held":[],"cwd":"/"}"#
            )
            .is_err()
        );
    }
}

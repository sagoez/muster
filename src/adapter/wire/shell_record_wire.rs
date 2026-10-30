use std::{ffi::OsString, os::unix::ffi::OsStringExt as _, path::PathBuf};

use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::WireForm;
use crate::domain::{
    occupancy::{LoadId, LoadStart, ReportSeq, ReportStamp, ShellId, ShellRecord},
    value::{EpochNanos, Pid, ProjectRoot, StartToken},
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
    /// When the hook load the report was made from began.
    #[getset(get_copy = "pub")]
    load: LoadStart,
    /// The pid that load spawned, breaking a tie of its start.
    #[getset(get_copy = "pub")]
    load_pid: Pid,
    /// The prompt the report was made at, within that load.
    #[getset(get_copy = "pub")]
    seq: ReportSeq,
    /// The folder the shell was in, as its raw bytes, so a name that is not
    /// UTF-8 travels whole.
    #[getset(get = "pub")]
    folder: Vec<u8>,
    /// When the shell was first seen in that folder, in nanoseconds.
    #[getset(get_copy = "pub")]
    arrived: EpochNanos,
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
            load: self.stamp().load().started(),
            load_pid: self.stamp().load().pid(),
            seq: self.stamp().seq(),
            folder: self.folder().clone().into_os_string().into_vec(),
            arrived: *self.arrived(),
            project: self.project().clone(),
            held: self.held().clone(),
        }
    }

    fn from_wire(wire: ShellRecordWire) -> Result<Self, String> {
        let load = LoadId::builder()
            .started(wire.load)
            .pid(wire.load_pid)
            .build();
        Ok(Self::builder()
            .shell(ShellId::builder().pid(wire.pid).start(wire.start).build())
            .stamp(ReportStamp::builder().load(load).seq(wire.seq).build())
            .folder(PathBuf::from(OsString::from_vec(wire.folder)))
            .arrived(wire.arrived)
            .project(wire.project)
            .held(wire.held)
            .build())
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt as _};

    use super::*;
    use crate::domain::occupancy::testing::stamp;

    /// The shell every test record is of.
    fn shell() -> ShellId {
        ShellId::builder()
            .pid(Pid::try_new(4242).unwrap())
            .start(StartToken::try_new(77).unwrap())
            .build()
    }

    /// A record is one flat object, its folder as bytes and its project
    /// `null` outside every project, and refuses a key it does not know.
    #[test]
    fn a_record_travels_flat_and_refuses_unknown_keys() {
        let root = |path: &str| ProjectRoot::try_new(PathBuf::from(path)).unwrap();
        let record = |project: Option<&str>, held: &[&str]| {
            ShellRecord::builder()
                .shell(shell())
                .stamp(stamp(9, 3))
                .folder(PathBuf::from("/a"))
                .arrived(EpochNanos::new(5))
                .project(project.map(root))
                .held(held.iter().map(|path| root(path)).collect())
                .build()
        };
        for (project, held, json) in [
            (
                Some("/home/me/shop/admin"),
                &["/home/me/shop"][..],
                r#"{"pid":4242,"start":77,"load":9,"load-pid":2,"seq":3,"folder":[47,97],"arrived":5,"project":"/home/me/shop/admin","held":["/home/me/shop"]}"#,
            ),
            (
                None,
                &[][..],
                r#"{"pid":4242,"start":77,"load":9,"load-pid":2,"seq":3,"folder":[47,97],"arrived":5,"project":null,"held":[]}"#,
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
                r#"{"pid":4242,"start":77,"load":9,"load-pid":2,"seq":3,"folder":[47],"arrived":5,"project":null,"held":[],"cwd":"/"}"#
            )
            .is_err()
        );
    }

    /// S-M1: a folder whose name is not UTF-8 is written and read back
    /// byte for byte.
    #[test]
    fn a_folder_that_is_not_utf8_round_trips() {
        let folder = PathBuf::from(OsStr::from_bytes(b"/tmp/bad\xffdir"));
        let record = ShellRecord::builder()
            .shell(shell())
            .stamp(stamp(9, 3))
            .folder(folder.clone())
            .arrived(EpochNanos::new(5))
            .build();

        let json = serde_json::to_string(&record.to_wire()).unwrap();
        let read: ShellRecordWire = serde_json::from_str(&json).unwrap();

        assert_eq!(ShellRecord::from_wire(read).unwrap().folder(), &folder);
    }
}

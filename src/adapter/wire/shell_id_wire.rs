use getset::CopyGetters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::WireForm;
use crate::domain::{
    occupancy::ShellId,
    value::{Pid, StartToken},
};

/// A shell, as its report to the service names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ShellIdWire {
    /// The shell's process id.
    pid: Pid,
    /// That process's start token.
    start: StartToken,
}

impl WireForm for ShellId {
    type Wire = ShellIdWire;

    fn to_wire(&self) -> ShellIdWire {
        ShellIdWire {
            pid: self.pid(),
            start: self.start(),
        }
    }

    fn from_wire(wire: ShellIdWire) -> Result<Self, String> {
        Ok(Self::builder().pid(wire.pid).start(wire.start).build())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shell travels as its pid and start token, and nothing else reads.
    #[test]
    fn a_shell_travels_as_its_pid_and_start() {
        let shell = ShellId::builder()
            .pid(Pid::try_new(4242).unwrap())
            .start(StartToken::try_new(77).unwrap())
            .build();
        let json = r#"{"pid":4242,"start":77}"#;

        assert_eq!(serde_json::to_string(&shell.to_wire()).unwrap(), json);
        let read: ShellIdWire = serde_json::from_str(json).unwrap();
        assert_eq!(ShellId::from_wire(read), Ok(shell));
        assert!(serde_json::from_str::<ShellIdWire>(r#"{"pid":4242,"start":77,"x":1}"#).is_err());
    }
}

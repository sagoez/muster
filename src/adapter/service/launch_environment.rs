use std::{collections::BTreeMap, env, ffi::OsString};

use nutype::nutype;

use crate::{
    adapter::herdr::PANE_IDENTITY_ENV,
    domain::value::{EnvName, EnvValue},
};

/// The environment of the command that asked the service to launch a process,
/// which the process runs with under its own variables. Muster's own `MUSTER_`
/// variables, the terminal workspace's pane identity, and anything not carried
/// as text, stay out.
#[nutype(
    default = BTreeMap::new(),
    derive(Debug, Clone, PartialEq, Eq, Default, AsRef, Serialize, Deserialize)
)]
pub struct LaunchEnvironment(BTreeMap<EnvName, EnvValue>);

impl LaunchEnvironment {
    /// The environment this process runs in, for a client asking for a launch.
    pub fn current() -> Self {
        Self::from_vars(env::vars_os())
    }

    /// The variables of `vars` a launch can carry: those named and valued as
    /// text, outside muster's own namespace and the pane identity.
    pub fn from_vars(vars: impl IntoIterator<Item = (OsString, OsString)>) -> Self {
        Self::new(
            vars.into_iter()
                .filter_map(|(name, value)| {
                    let name = name.into_string().ok()?;
                    if PANE_IDENTITY_ENV.contains(&name.as_str()) {
                        return None;
                    }
                    let name = EnvName::try_new(name).ok()?;
                    let value = EnvValue::try_new(value.into_string().ok()?).ok()?;
                    Some((name, value))
                })
                .collect(),
        )
    }

    /// The variables as a child's environment takes them.
    pub fn to_os(&self) -> BTreeMap<OsString, OsString> {
        self.as_ref()
            .iter()
            .map(|(name, value)| {
                (
                    OsString::from(name.as_ref()),
                    OsString::from(value.as_ref()),
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn muster_variables_stay_out_and_the_rest_survive_the_wire() {
        let launch = LaunchEnvironment::from_vars([
            ("PATH".into(), "/usr/bin".into()),
            ("MUSTER_PROJECT".into(), "/elsewhere".into()),
            ("EMPTY".into(), "".into()),
        ]);

        let json = serde_json::to_string(&launch).unwrap();
        let read: LaunchEnvironment = serde_json::from_str(&json).unwrap();

        assert_eq!(read, launch);
        assert_eq!(
            read.to_os(),
            BTreeMap::from([
                (OsString::from("EMPTY"), OsString::new()),
                (OsString::from("PATH"), OsString::from("/usr/bin")),
            ])
        );
    }

    /// Security M1: a launch from a herdr pane never carries the pane's
    /// identity, so a process it starts is never taken to run in that pane;
    /// how to reach herdr itself survives.
    #[test]
    fn the_pane_identity_stays_out_of_a_launch() {
        let launch = LaunchEnvironment::from_vars(
            [
                ("HERDR_PANE_ID", "w1:p3"),
                ("HERDR_PANE_RUNTIME_ID", "r7"),
                ("HERDR_TAB_ID", "w1:t1"),
                ("HERDR_WORKSPACE_ID", "w1"),
                ("HERDR_SOCKET_PATH", "/run/herdr.sock"),
            ]
            .map(|(name, value)| (name.into(), value.into())),
        );

        assert_eq!(
            launch.to_os(),
            BTreeMap::from([(
                OsString::from("HERDR_SOCKET_PATH"),
                OsString::from("/run/herdr.sock")
            )])
        );
    }
}

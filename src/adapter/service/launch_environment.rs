use std::{collections::BTreeMap, env, ffi::OsString};

use nutype::nutype;

use crate::domain::value::{EnvName, EnvValue};

/// The environment of the command that asked the service to launch a process,
/// which the process runs with under its own variables. Muster's own `MUSTER_`
/// variables, and anything not carried as text, stay out.
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
    /// text, outside muster's own namespace.
    pub fn from_vars(vars: impl IntoIterator<Item = (OsString, OsString)>) -> Self {
        Self::new(
            vars.into_iter()
                .filter_map(|(name, value)| {
                    let name = EnvName::try_new(name.into_string().ok()?).ok()?;
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
}

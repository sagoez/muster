use std::fmt::{self, Display, Formatter};

use super::{OverCap, UnconfinedDir, environment::ENV_VARS_MAX};
use crate::domain::value::{
    ENV_NAME_MAX_CHARS, ENV_VALUE_MAX_CHARS, EnvName, PROCESS_NAME_MAX_CHARS, ProcessName,
};

/// What every over-cap problem says follows from it.
const NEVER_STARTED: &str = "so it is never started";

/// Why a stored process reads but is never started until `muster set` or
/// `muster rm` repairs it; only a hand edit, or a write from before a cap,
/// stores one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredProblem {
    /// Its working directory leaves the project root.
    UnconfinedDir(UnconfinedDir),
    /// A value of it runs past a cap a write is held to.
    OverCap {
        /// The process.
        name: ProcessName,
        /// The cap it runs past.
        over: OverCap,
    },
}

/// `var` quoted so pasting it into a shell gives it back.
fn quoted(var: &EnvName) -> String {
    shlex::try_quote(var.as_ref()).map_or_else(|_| var.to_string(), |quoted| quoted.into_owned())
}

impl Display for StoredProblem {
    /// Which process, what is wrong with it, and the commands that fix it.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let (name, over) = match self {
            Self::UnconfinedDir(unconfined) => return unconfined.fmt(f),
            Self::OverCap { name, over } => (name.shell_quoted(), over),
        };
        let remove = format!("`muster rm {name}`");
        match over {
            OverCap::Name { found } => write!(
                f,
                "the stored name of {name} is {found} characters, more than the \
                 {PROCESS_NAME_MAX_CHARS} muster allows, {NEVER_STARTED}; {remove} fixes it"
            ),
            OverCap::VarName { name: var, found } => write!(
                f,
                "a stored variable name of {name} is {found} characters, more than the \
                 {ENV_NAME_MAX_CHARS} muster allows, {NEVER_STARTED}; \
                 `muster set {name} --unset-env {var}` or {remove} fixes it",
                var = quoted(var)
            ),
            OverCap::VarValue { name: var, found } => write!(
                f,
                "the stored variable {var} of {name} is {found} characters, more than the \
                 {ENV_VALUE_MAX_CHARS} muster allows, {NEVER_STARTED}; \
                 `muster set {name} --unset-env {var}` or {remove} fixes it",
                var = quoted(var)
            ),
            OverCap::Vars { found } => write!(
                f,
                "{name} has {found} stored variables, more than the {ENV_VARS_MAX} muster \
                 allows, {NEVER_STARTED}; unsetting {extra} of them with \
                 `muster set {name} --unset-env <NAME>` or {remove} fixes it",
                extra = found - ENV_VARS_MAX
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each stored problem names the process, the cap and a command that
    /// fixes it.
    #[test]
    fn each_problem_names_the_process_and_the_fix() {
        let web = ProcessName::try_new("web").unwrap();
        let cert = EnvName::try_new("CERT").unwrap();
        let shown = |over: OverCap| {
            StoredProblem::OverCap {
                name: web.clone(),
                over,
            }
            .to_string()
        };

        let value = shown(OverCap::VarValue {
            name: cert.clone(),
            found: ENV_VALUE_MAX_CHARS + 1,
        });
        assert!(value.contains("variable CERT of 'web'"), "{value}");
        assert!(
            value.contains("`muster set 'web' --unset-env CERT`"),
            "{value}"
        );
        let vars = shown(OverCap::Vars {
            found: ENV_VARS_MAX + 2,
        });
        assert!(vars.contains("unsetting 2 of them"), "{vars}");
        let long = shown(OverCap::Name {
            found: PROCESS_NAME_MAX_CHARS + 1,
        });
        assert!(long.contains("`muster rm 'web'` fixes it"), "{long}");
        assert!(!long.contains("muster set"), "{long}");
        let var_name = shown(OverCap::VarName {
            name: cert,
            found: ENV_NAME_MAX_CHARS + 1,
        });
        assert!(var_name.contains("--unset-env CERT"), "{var_name}");
    }
}

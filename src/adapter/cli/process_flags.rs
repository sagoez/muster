use super::{
    args::{positive_duration, restart_parser, stop_signal_parser},
    error::CliError,
};
use crate::domain::{
    process::{RestartPolicy, StopSignal},
    value::{EnvName, EnvValue, PositiveDuration},
};

/// Separates a variable's name from its value in `--env KEY=VALUE`.
const ENV_SEPARATOR: char = '=';

/// The definition flags `add` and `set` share; each one left out keeps the
/// default (`add`) or the current value (`set`).
#[derive(clap::Args)]
pub struct ProcessFlags {
    /// Set an environment variable, `KEY=VALUE` (repeatable).
    #[arg(long = "env", value_name = "KEY=VALUE")]
    pub env: Vec<String>,
    /// Start it with `muster up`.
    #[arg(long, conflicts_with = "no_autostart")]
    pub autostart: bool,
    /// Leave it out of `muster up`.
    #[arg(long)]
    pub no_autostart: bool,
    /// What happens when its child exits.
    #[arg(long, value_name = "POLICY", value_parser = restart_parser())]
    pub restart: Option<RestartPolicy>,
    /// The signal a graceful stop sends.
    #[arg(long, value_name = "SIGNAL", value_parser = stop_signal_parser())]
    pub stop_signal: Option<StopSignal>,
    /// How long a graceful stop waits before it kills, such as `5s`.
    #[arg(long, value_name = "DURATION", value_parser = positive_duration)]
    pub stop_grace: Option<PositiveDuration>,
}

impl ProcessFlags {
    /// Whether `muster up` starts it, when a flag said.
    pub fn autostart(&self) -> Option<bool> {
        match (self.autostart, self.no_autostart) {
            (true, _) => Some(true),
            (false, true) => Some(false),
            (false, false) => None,
        }
    }

    /// The `--env` variables, in the order given.
    ///
    /// # Errors
    /// Returns [`CliError::InvalidEnv`] naming the first that is not a
    /// usable `KEY=VALUE`.
    pub fn environment(&self) -> Result<Vec<(EnvName, EnvValue)>, CliError> {
        self.env.iter().map(|raw| env_assignment(raw)).collect()
    }

    /// Whether no flag was given.
    pub fn is_empty(&self) -> bool {
        self.env.is_empty()
            && self.autostart().is_none()
            && self.restart.is_none()
            && self.stop_signal.is_none()
            && self.stop_grace.is_none()
    }
}

/// One `KEY=VALUE` assignment.
///
/// # Errors
/// Returns [`CliError::InvalidEnv`] when it has no `=`, or its name or value
/// is not allowed.
fn env_assignment(raw: &str) -> Result<(EnvName, EnvValue), CliError> {
    let invalid = || CliError::InvalidEnv(raw.to_string());
    let (name, value) = raw.split_once(ENV_SEPARATOR).ok_or_else(invalid)?;
    Ok((
        EnvName::try_new(name).map_err(|_| invalid())?,
        EnvValue::try_new(value).map_err(|_| invalid())?,
    ))
}

/// The variable name `raw`, as `--unset-env` gives it.
///
/// # Errors
/// Returns [`CliError::InvalidEnv`] when it is not a usable name.
pub fn env_name(raw: &str) -> Result<EnvName, CliError> {
    EnvName::try_new(raw).map_err(|_| CliError::InvalidEnv(raw.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A value may hold `=` and may be empty; a name may not be empty,
    /// hold `=`, or start with `MUSTER_`.
    #[test]
    fn assignments_split_at_the_first_equals() {
        let (name, value) = env_assignment("URL=a=b").unwrap();
        assert_eq!((name.as_ref(), value.as_ref()), ("URL", "a=b"));
        assert_eq!(env_assignment("EMPTY=").unwrap().1.as_ref(), "");
        for raw in ["NOVALUE", "=x", "MUSTER_PROJECT=/p", "A=b\0c"] {
            assert!(
                matches!(env_assignment(raw), Err(CliError::InvalidEnv(_))),
                "{raw}"
            );
        }
    }
}

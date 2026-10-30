use getset::CopyGetters;

use super::{
    args::{positive_duration, restart_parser, stop_grace, stop_signal_parser},
    error::CliError,
};
use crate::{
    adapter::holder::LineMatcher,
    domain::{
        config::{ConfigError, DeclaredPorts},
        process::{PatternKind, ReadyCheck, ReadyProbe, RestartPolicy, StopSignal},
        value::{EnvName, EnvValue, LocalPort, LocalUrl, PositiveDuration, StopGrace},
    },
};

/// Separates a variable's name from its value in `--env KEY=VALUE`.
const ENV_SEPARATOR: char = '=';
/// The clap group of the `--ready-*` probe flags, at most one of which is
/// given.
pub const READY_GROUP: &str = "ready";

/// The definition flags `add` and `set` share; each one left out keeps the
/// default (`add`) or the current value (`set`).
#[derive(clap::Args, CopyGetters)]
pub struct ProcessFlags {
    /// Set an environment variable, `KEY=VALUE` (repeatable).
    #[arg(long = "env", value_name = "KEY=VALUE")]
    env: Vec<String>,
    /// Start it with `muster up`.
    #[arg(long, conflicts_with = "no_autostart")]
    autostart: bool,
    /// Leave it out of `muster up`.
    #[arg(long)]
    no_autostart: bool,
    /// What happens when its child exits.
    #[arg(long, value_name = "POLICY", value_parser = restart_parser())]
    #[getset(get_copy = "pub")]
    restart: Option<RestartPolicy>,
    /// The signal a graceful stop sends.
    #[arg(long, value_name = "SIGNAL", value_parser = stop_signal_parser())]
    #[getset(get_copy = "pub")]
    stop_signal: Option<StopSignal>,
    /// How long a graceful stop waits before it kills, such as `5s`; at most
    /// 1h.
    #[arg(long, value_name = "DURATION", value_parser = stop_grace)]
    #[getset(get_copy = "pub")]
    stop_grace: Option<StopGrace>,
    /// Ready once an output line contains this text; needs --ready-timeout.
    #[arg(long, group = READY_GROUP, value_name = "TEXT", requires = "ready_timeout")]
    ready_log: Option<String>,
    /// Read --ready-log as a regular expression.
    #[arg(long, requires = "ready_log")]
    ready_regex: bool,
    /// Ready once this localhost port accepts a connection; needs
    /// --ready-timeout.
    #[arg(long, group = READY_GROUP, value_name = "PORT", requires = "ready_timeout")]
    ready_port: Option<LocalPort>,
    /// Ready once this localhost URL answers below 500; needs
    /// --ready-timeout.
    #[arg(long, group = READY_GROUP, value_name = "URL", requires = "ready_timeout")]
    ready_http: Option<LocalUrl>,
    /// How long each run may take to become ready, such as `60s`.
    #[arg(long, requires = READY_GROUP, value_name = "DURATION", value_parser = positive_duration)]
    ready_timeout: Option<PositiveDuration>,
    /// A TCP port it listens on, checked free before each start
    /// (repeatable); on `set`, the ports given replace the stored ones.
    #[arg(long = "port", value_name = "PORT")]
    port: Vec<LocalPort>,
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

    /// The readiness check the `--ready-*` flags describe, when they name
    /// one; a pattern is compiled before it is stored.
    ///
    /// # Errors
    /// Returns [`CliError::InvalidPattern`] for a blank or overlong
    /// `--ready-log`, or a `--ready-regex` that does not compile.
    pub fn ready(&self) -> Result<Option<ReadyCheck>, CliError> {
        let Some(timeout) = self.ready_timeout else {
            return Ok(None);
        };
        let probe = match (&self.ready_log, self.ready_port, &self.ready_http) {
            (Some(text), ..) => ReadyProbe::Log(LineMatcher::checked(
                text,
                PatternKind::of_regex(self.ready_regex),
            )?),
            (None, Some(port), _) => ReadyProbe::Port(port),
            (None, None, Some(url)) => ReadyProbe::Http(url.clone()),
            (None, None, None) => return Ok(None),
        };
        Ok(Some(
            ReadyCheck::builder().probe(probe).timeout(timeout).build(),
        ))
    }

    /// The `--port` ports, when any was given.
    ///
    /// # Errors
    /// Returns [`ConfigError::RepeatedPort`] for a port given twice, and
    /// [`ConfigError::TooManyPorts`] past
    /// [`DECLARED_PORTS_MAX`](crate::domain::config::DECLARED_PORTS_MAX).
    pub fn ports(&self) -> Result<Option<DeclaredPorts>, ConfigError> {
        if self.port.is_empty() {
            return Ok(None);
        }
        DeclaredPorts::listed(self.port.iter().copied()).map(Some)
    }

    /// Whether no flag was given.
    pub fn is_empty(&self) -> bool {
        self.port.is_empty()
            && self.env.is_empty()
            && self.autostart().is_none()
            && self.restart.is_none()
            && self.stop_signal.is_none()
            && self.stop_grace.is_none()
            && self.ready_timeout.is_none()
    }
}

/// One `KEY=VALUE` assignment. Its caps are held by the write that stores
/// it, which names the variable.
///
/// # Errors
/// Returns [`CliError::InvalidEnv`] when it has no `=`, or its name or value
/// is not allowed.
fn env_assignment(raw: &str) -> Result<(EnvName, EnvValue), CliError> {
    let (name, value) = raw
        .split_once(ENV_SEPARATOR)
        .ok_or_else(|| CliError::InvalidEnv(raw.to_string()))?;
    let value = EnvValue::try_new(value).map_err(|_| CliError::InvalidEnv(raw.to_string()))?;
    Ok((env_name(name)?, value))
}

/// The variable name `raw`, as `--unset-env` gives it: never held to the
/// name cap, so a variable stored before the cap can still be unset.
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

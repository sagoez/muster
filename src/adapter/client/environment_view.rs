use std::collections::BTreeMap;

use serde::Serialize;

use crate::domain::config::Environment;

/// A process's variables as a definition shows them: with their values for
/// the user, by name alone for an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum EnvironmentView {
    /// Each variable's value, by name.
    Values(BTreeMap<String, String>),
    /// The variables' names, sorted.
    Names(Vec<String>),
}

impl EnvironmentView {
    /// Every variable of `environment` with its value.
    pub fn for_values(environment: &Environment) -> Self {
        Self::Values(
            environment
                .vars()
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
        )
    }

    /// The names of `environment`'s variables, never their values.
    pub fn for_names(environment: &Environment) -> Self {
        Self::Names(environment.vars().keys().map(ToString::to_string).collect())
    }
}

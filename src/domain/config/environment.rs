use std::collections::BTreeMap;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::value::{EnvName, EnvValue};

/// The variables a process is given on top of the environment it inherits,
/// kept sorted by name.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Environment {
    /// Each variable's value, by name.
    vars: BTreeMap<EnvName, EnvValue>,
}

impl Environment {
    /// An environment with no variables.
    pub fn empty() -> Self {
        Self {
            vars: BTreeMap::new(),
        }
    }

    /// This environment with `name` set to `value`, replacing any earlier value.
    pub fn with_var(mut self, name: EnvName, value: EnvValue) -> Self {
        self.vars.insert(name, value);
        self
    }

    /// This environment without `name`; absent already is a no-op.
    pub fn without_var(mut self, name: &EnvName) -> Self {
        self.vars.remove(name);
        self
    }

    /// Whether no variable is set.
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }
}

impl FromIterator<(EnvName, EnvValue)> for Environment {
    /// Collects variables, a later value for a name replacing an earlier one.
    fn from_iter<I: IntoIterator<Item = (EnvName, EnvValue)>>(vars: I) -> Self {
        Self {
            vars: vars.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(raw: &str) -> EnvName {
        EnvName::try_new(raw).unwrap()
    }

    fn value(raw: &str) -> EnvValue {
        EnvValue::try_new(raw).unwrap()
    }

    #[test]
    fn sets_replaces_and_removes_variables() {
        let environment = Environment::empty()
            .with_var(name("PORT"), value("5173"))
            .with_var(name("PORT"), value("8080"))
            .with_var(name("HOST"), value("localhost"))
            .without_var(&name("HOST"))
            .without_var(&name("ABSENT"));

        assert_eq!(environment.vars().len(), 1);
        assert_eq!(environment.vars()[&name("PORT")], value("8080"));
        assert!(!environment.is_empty());
        assert!(Environment::empty().is_empty());
    }
}

use std::collections::BTreeMap;

use getset::Getters;

use super::over_cap::OverCap;
use crate::domain::value::{EnvName, EnvValue};

/// Most variables a write leaves one process with, so one definition cannot
/// flood every listing that names them.
pub const ENV_VARS_MAX: usize = 64;

/// The variables a process is given on top of the environment it inherits,
/// kept sorted by name. A stored one may run past a cap, so the caps are
/// checked by [`Environment::over_cap`] on a write, never on a read.
#[derive(Clone, Debug, PartialEq, Eq, Getters)]
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

    /// The environment holding `vars`, a later value for a name replacing an
    /// earlier one.
    pub fn of(vars: impl IntoIterator<Item = (EnvName, EnvValue)>) -> Self {
        Self {
            vars: vars.into_iter().collect(),
        }
    }

    /// This environment with `name` set to `value`, replacing any earlier value.
    pub fn with_var(mut self, name: EnvName, value: EnvValue) -> Self {
        self.vars.insert(name, value);
        self
    }

    /// This environment with `set` set and then `unset` removed, so a name in
    /// both ends up unset.
    pub fn changed<'a>(
        mut self,
        set: impl IntoIterator<Item = (EnvName, EnvValue)>,
        unset: impl IntoIterator<Item = &'a EnvName>,
    ) -> Self {
        self.vars.extend(set);
        for name in unset {
            self.vars.remove(name);
        }
        self
    }

    /// The first cap this environment runs past: a variable's name or
    /// value, in name order, then the number of variables.
    pub fn over_cap(&self) -> Option<OverCap> {
        self.vars
            .iter()
            .find_map(|(name, value)| OverCap::of_var(name, value))
            .or_else(|| OverCap::of_var_count(self.vars.len()))
    }

    /// Whether no variable is set.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value::ENV_VALUE_MAX_CHARS;

    /// The variable name `raw`.
    fn name(raw: &str) -> EnvName {
        EnvName::try_new(raw).unwrap()
    }

    /// The variable value `raw`.
    fn value(raw: &str) -> EnvValue {
        EnvValue::try_new(raw).unwrap()
    }

    /// `count` distinct variables.
    fn vars(count: usize) -> Vec<(EnvName, EnvValue)> {
        (0..count)
            .map(|index| (name(&format!("V{index}")), value("1")))
            .collect()
    }

    #[test]
    fn sets_replaces_and_removes_variables() {
        let environment = Environment::empty()
            .with_var(name("PORT"), value("5173"))
            .changed(
                [
                    (name("PORT"), value("8080")),
                    (name("HOST"), value("localhost")),
                ],
                [&name("HOST"), &name("ABSENT")],
            );

        assert_eq!(environment.vars().len(), 1);
        assert_eq!(environment.vars()[&name("PORT")], value("8080"));
        assert!(!environment.is_empty());
        assert!(Environment::empty().is_empty());
    }

    /// An environment past a cap still holds every variable, and names the
    /// first cap it runs past: a value before the count.
    #[test]
    fn an_environment_past_a_cap_holds_and_names_it() {
        assert_eq!(Environment::of(vars(ENV_VARS_MAX)).over_cap(), None);
        let crowded = Environment::of(vars(ENV_VARS_MAX + 1));
        assert_eq!(crowded.vars().len(), ENV_VARS_MAX + 1);
        assert_eq!(
            crowded.over_cap(),
            Some(OverCap::Vars {
                found: ENV_VARS_MAX + 1
            })
        );

        let long = crowded.with_var(name("CERT"), value(&"v".repeat(ENV_VALUE_MAX_CHARS + 1)));
        assert!(matches!(
            long.over_cap(),
            Some(OverCap::VarValue { name: var, .. }) if var == name("CERT")
        ));
    }
}

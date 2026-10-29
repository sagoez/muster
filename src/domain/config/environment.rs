use std::collections::BTreeMap;

use getset::Getters;

use super::too_many_vars::TooManyVars;
use crate::domain::value::{EnvName, EnvValue};

/// Most variables one process is given, so one definition cannot flood every
/// listing that names them.
pub const ENV_VARS_MAX: usize = 64;

/// The variables a process is given on top of the environment it inherits,
/// kept sorted by name, at most [`ENV_VARS_MAX`] of them.
/// No builder: every way in is checked against the bound.
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
    ///
    /// # Errors
    /// Returns [`TooManyVars`] when they name more than [`ENV_VARS_MAX`]
    /// variables.
    pub fn of(vars: impl IntoIterator<Item = (EnvName, EnvValue)>) -> Result<Self, TooManyVars> {
        Self::checked(vars.into_iter().collect())
    }

    /// This environment with `name` set to `value`, replacing any earlier value.
    ///
    /// # Errors
    /// Returns [`TooManyVars`] when a new name would take it past
    /// [`ENV_VARS_MAX`].
    pub fn with_var(self, name: EnvName, value: EnvValue) -> Result<Self, TooManyVars> {
        self.changed([(name, value)], [])
    }

    /// This environment with `set` set and then `unset` removed, so a name in
    /// both ends up unset; only the result is held to the bound.
    ///
    /// # Errors
    /// Returns [`TooManyVars`] when the result holds more than
    /// [`ENV_VARS_MAX`] variables.
    pub fn changed<'a>(
        mut self,
        set: impl IntoIterator<Item = (EnvName, EnvValue)>,
        unset: impl IntoIterator<Item = &'a EnvName>,
    ) -> Result<Self, TooManyVars> {
        self.vars.extend(set);
        for name in unset {
            self.vars.remove(name);
        }
        Self::checked(self.vars)
    }

    /// `vars` as an environment, refused past [`ENV_VARS_MAX`].
    fn checked(vars: BTreeMap<EnvName, EnvValue>) -> Result<Self, TooManyVars> {
        if vars.len() > ENV_VARS_MAX {
            return Err(TooManyVars::builder().found(vars.len()).build());
        }
        Ok(Self { vars })
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
            .unwrap()
            .changed(
                [
                    (name("PORT"), value("8080")),
                    (name("HOST"), value("localhost")),
                ],
                [&name("HOST"), &name("ABSENT")],
            )
            .unwrap();

        assert_eq!(environment.vars().len(), 1);
        assert_eq!(environment.vars()[&name("PORT")], value("8080"));
        assert!(!environment.is_empty());
        assert!(Environment::empty().is_empty());
    }

    /// Up to the maximum variables are kept; one more is refused however it
    /// arrives, while replacing a value at the maximum still works.
    #[test]
    fn the_variable_count_is_bounded() {
        let full = Environment::of(vars(ENV_VARS_MAX)).unwrap();
        let refused = TooManyVars::builder().found(ENV_VARS_MAX + 1).build();

        assert_eq!(Environment::of(vars(ENV_VARS_MAX + 1)), Err(refused));
        assert_eq!(
            full.clone().with_var(name("EXTRA"), value("1")),
            Err(refused)
        );
        assert!(full.clone().with_var(name("V0"), value("2")).is_ok());
        assert!(
            full.changed([(name("EXTRA"), value("1"))], [&name("V0")])
                .is_ok()
        );
    }
}

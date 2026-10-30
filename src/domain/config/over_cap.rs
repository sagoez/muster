use thiserror::Error;

use super::{
    declared_ports::{DECLARED_PORTS_MAX, DeclaredPorts},
    environment::ENV_VARS_MAX,
};
use crate::domain::value::{
    ENV_NAME_MAX_CHARS, ENV_VALUE_MAX_CHARS, EnvName, EnvValue, PROCESS_NAME_MAX_CHARS,
    ProcessName, refused_name,
};

/// A value past a cap every write is held to. A value stored before its cap
/// still reads; it only keeps its process from starting.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OverCap {
    /// The process name is too long.
    #[error("the name is {found} characters; at most {PROCESS_NAME_MAX_CHARS} are allowed")]
    Name {
        /// How many characters it holds.
        found: usize,
    },
    /// A variable's name is too long.
    #[error(
        "variable name '{}' is {found} characters; at most {ENV_NAME_MAX_CHARS} are allowed",
        refused_name(name.as_ref())
    )]
    VarName {
        /// The name.
        name: EnvName,
        /// How many characters it holds.
        found: usize,
    },
    /// A variable's value is too long.
    #[error("variable '{name}' is {found} characters; at most {ENV_VALUE_MAX_CHARS} are allowed")]
    VarValue {
        /// The variable whose value it is.
        name: EnvName,
        /// How many characters the value holds.
        found: usize,
    },
    /// The process has too many variables.
    #[error("{found} variables are more than the {ENV_VARS_MAX} a process may have")]
    Vars {
        /// How many it has.
        found: usize,
    },
    /// The process declares too many ports.
    #[error("{found} ports are more than the {DECLARED_PORTS_MAX} a process may declare")]
    Ports {
        /// How many it declares.
        found: usize,
    },
}

impl OverCap {
    /// The cap `name` runs past, if any.
    pub fn of_name(name: &ProcessName) -> Option<Self> {
        let found = name.char_count();
        (found > PROCESS_NAME_MAX_CHARS).then_some(Self::Name { found })
    }

    /// The cap the variable `name` set to `value` runs past, its name's
    /// checked first.
    pub fn of_var(name: &EnvName, value: &EnvValue) -> Option<Self> {
        let name_chars = name.as_ref().chars().count();
        if name_chars > ENV_NAME_MAX_CHARS {
            return Some(Self::VarName {
                name: name.clone(),
                found: name_chars,
            });
        }
        let found = value.as_ref().chars().count();
        (found > ENV_VALUE_MAX_CHARS).then(|| Self::VarValue {
            name: name.clone(),
            found,
        })
    }

    /// The cap `count` variables run past, if any.
    pub fn of_var_count(count: usize) -> Option<Self> {
        (count > ENV_VARS_MAX).then_some(Self::Vars { found: count })
    }

    /// The cap `ports` runs past, if any.
    pub fn of_ports(ports: &DeclaredPorts) -> Option<Self> {
        let found = ports.len();
        (found > DECLARED_PORTS_MAX).then_some(Self::Ports { found })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value::LocalPort;

    /// The variable name `raw`.
    fn name(raw: &str) -> EnvName {
        EnvName::try_new(raw).unwrap()
    }

    /// The variable value `raw`.
    fn value(raw: &str) -> EnvValue {
        EnvValue::try_new(raw).unwrap()
    }

    /// Each cap holds its maximum and refuses one more, naming the count.
    #[test]
    fn each_cap_keeps_its_maximum_and_refuses_one_more() {
        let process = |chars: usize| ProcessName::try_new("n".repeat(chars)).unwrap();
        assert_eq!(OverCap::of_name(&process(PROCESS_NAME_MAX_CHARS)), None);
        assert_eq!(
            OverCap::of_name(&process(PROCESS_NAME_MAX_CHARS + 1)),
            Some(OverCap::Name {
                found: PROCESS_NAME_MAX_CHARS + 1
            })
        );

        let long_name = name(&"N".repeat(ENV_NAME_MAX_CHARS + 1));
        assert_eq!(
            OverCap::of_var(&name(&"N".repeat(ENV_NAME_MAX_CHARS)), &value("1")),
            None
        );
        assert!(matches!(
            OverCap::of_var(&long_name, &value(&"v".repeat(ENV_VALUE_MAX_CHARS + 1))),
            Some(OverCap::VarName { found, .. }) if found == ENV_NAME_MAX_CHARS + 1
        ));
        assert_eq!(
            OverCap::of_var(&name("CERT"), &value(&"v".repeat(ENV_VALUE_MAX_CHARS))),
            None
        );

        assert_eq!(OverCap::of_var_count(ENV_VARS_MAX), None);
        assert_eq!(
            OverCap::of_var_count(ENV_VARS_MAX + 1),
            Some(OverCap::Vars {
                found: ENV_VARS_MAX + 1
            })
        );

        let ports = |count: usize| {
            DeclaredPorts::stored(
                (1..=count)
                    .map(|port| LocalPort::try_new(u16::try_from(port).unwrap()).unwrap())
                    .collect(),
            )
        };
        assert_eq!(OverCap::of_ports(&ports(DECLARED_PORTS_MAX)), None);
        assert_eq!(
            OverCap::of_ports(&ports(DECLARED_PORTS_MAX + 1)),
            Some(OverCap::Ports {
                found: DECLARED_PORTS_MAX + 1
            })
        );
    }

    /// A refusal names the variable and reads as a sentence, never as the
    /// validator's terms.
    #[test]
    fn a_refusal_names_the_variable_plainly() {
        let refused =
            OverCap::of_var(&name("CERT"), &value(&"v".repeat(ENV_VALUE_MAX_CHARS + 1))).unwrap();

        assert_eq!(
            refused.to_string(),
            format!(
                "variable 'CERT' is {} characters; at most {ENV_VALUE_MAX_CHARS} are allowed",
                ENV_VALUE_MAX_CHARS + 1
            )
        );
    }
}

use strum::{Display, EnumIter, EnumString};

/// A shell muster's prompt hook is written for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString, EnumIter)]
#[strum(serialize_all = "kebab-case")]
pub enum HookShell {
    /// GNU bash 3.2 or later.
    Bash,
    /// zsh 5 or later.
    Zsh,
    /// fish 3 or later.
    Fish,
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    #[test]
    fn each_shell_is_named_in_kebab_case_and_parses_back() {
        let names: Vec<String> = HookShell::iter().map(|shell| shell.to_string()).collect();
        assert_eq!(names, ["bash", "zsh", "fish"]);
        for shell in HookShell::iter() {
            assert_eq!(shell.to_string().parse::<HookShell>().unwrap(), shell);
        }
    }
}

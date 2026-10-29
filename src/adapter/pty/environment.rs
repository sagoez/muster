use std::{
    collections::BTreeMap,
    env,
    ffi::{OsStr, OsString},
};

/// Variables a child keeps from the environment muster itself runs in.
const BASELINE_VARS: &[&str] = &["HOME", "USER", "LOGNAME", "SHELL", "PATH", "LANG", "TERM"];
/// Prefixes of the variable families a child keeps from muster's own
/// environment: locale categories and XDG base directories.
const BASELINE_PREFIXES: &[&str] = &["LC_", "XDG_"];

/// The part of this process's environment every child starts from: identity,
/// shell, search path, locale, terminal type and XDG directories. Anything
/// else muster happened to inherit, its own variables included, stays out.
pub fn baseline_environment() -> BTreeMap<OsString, OsString> {
    env::vars_os()
        .filter(|(name, _)| is_baseline(name))
        .collect()
}

/// Whether `name` is one of the variables [`baseline_environment`] keeps.
fn is_baseline(name: &OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        BASELINE_VARS.contains(&name)
            || BASELINE_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_baseline_keeps_identity_locale_and_xdg_and_nothing_else() {
        for kept in ["HOME", "PATH", "TERM", "LANG", "LC_ALL", "XDG_RUNTIME_DIR"] {
            assert!(is_baseline(OsStr::new(kept)), "{kept}");
        }
        for dropped in [
            "MUSTER_PROJECT",
            "AWS_SECRET_ACCESS_KEY",
            "PATHS",
            "HOMEDIR",
        ] {
            assert!(!is_baseline(OsStr::new(dropped)), "{dropped}");
        }
    }
}

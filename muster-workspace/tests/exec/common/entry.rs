use getset::{Getters, WithSetters};

/// The stop grace every test process has unless it says otherwise.
const GRACE: &str = "5s";
/// The restart policy every test process has unless it says otherwise.
const NEVER: &str = "never";

/// One process of a stack file.
#[derive(Getters, WithSetters)]
pub struct Entry<'a> {
    /// Its name.
    #[getset(get = "pub")]
    name: &'a str,
    /// The shell command it runs.
    #[getset(get = "pub")]
    command: &'a str,
    /// Whether `muster up` starts it.
    #[getset(get = "pub")]
    autostart: bool,
    /// Its restart policy, as the file spells it.
    #[getset(get = "pub", set_with = "pub")]
    restart: &'a str,
    /// Its stop grace, as the file spells it.
    #[getset(get = "pub", set_with = "pub")]
    grace: &'a str,
}

/// A process that autostarts unless `autostart` says otherwise, never
/// restarts, and stops with the default grace.
pub const fn entry<'a>(name: &'a str, command: &'a str, autostart: bool) -> Entry<'a> {
    Entry {
        name,
        command,
        autostart,
        restart: NEVER,
        grace: GRACE,
    }
}

/// The stack file holding `entries`, every value quoted so commands with
/// shell punctuation read back as they are.
pub fn stack(entries: &[Entry<'_>]) -> String {
    let quoted = |text: &str| serde_json::to_string(text).unwrap();
    let mut file = String::from("version: 1\nprocesses:\n");
    for entry in entries {
        file.push_str(&format!(
            "  - name: {}\n    command: {}\n    autostart: {}\n    restart: {}\n    stop:\n      signal: terminate\n      grace_period: {}\n",
            quoted(entry.name()),
            quoted(entry.command()),
            entry.autostart(),
            entry.restart(),
            entry.grace(),
        ));
    }
    file
}

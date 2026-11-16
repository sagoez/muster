use strum::Display;

/// A part of a hooks file muster's hook goes in, as a refusal names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display)]
pub enum HookFilePart {
    /// The whole file's value.
    #[strum(to_string = "a top level")]
    TopLevel,
    /// The hooks object, by event.
    #[strum(to_string = "a \"hooks\" entry")]
    Hooks,
    /// The session-start event's list of hook groups.
    #[strum(to_string = "a \"hooks\".\"SessionStart\" entry")]
    SessionStart,
}

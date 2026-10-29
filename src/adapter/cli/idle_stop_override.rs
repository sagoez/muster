use crate::domain::value::IdleStop;

/// The word that clears a project's idle stop, so it follows the setting.
pub const INHERIT_WORD: &str = "default";

/// What `muster projects set --idle-stop` gives a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleStopOverride {
    /// An idle stop of its own.
    Own(IdleStop),
    /// None of its own: it follows the `idle-stop` setting.
    Setting,
}

impl IdleStopOverride {
    /// The project's own idle stop to store, or none to follow the setting.
    pub fn own(self) -> Option<IdleStop> {
        match self {
            Self::Own(idle_stop) => Some(idle_stop),
            Self::Setting => None,
        }
    }
}

/// Parses `--idle-stop`: [`INHERIT_WORD`], or an idle stop such as `30m` or
/// `never`.
///
/// # Errors
/// Returns the form expected for anything else.
pub fn idle_stop_override(raw: &str) -> Result<IdleStopOverride, String> {
    if raw.trim() == INHERIT_WORD {
        return Ok(IdleStopOverride::Setting);
    }
    raw.parse()
        .map(IdleStopOverride::Own)
        .map_err(|error| format!("{error}; or {INHERIT_WORD} to follow the setting"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A duration and `never` are the project's own; `default` follows the
    /// setting; anything else names every form.
    #[test]
    fn an_override_reads_a_duration_never_or_default() {
        assert_eq!(
            idle_stop_override("10m"),
            Ok(IdleStopOverride::Own("10m".parse().unwrap()))
        );
        assert_eq!(
            idle_stop_override("never"),
            Ok(IdleStopOverride::Own(IdleStop::Never))
        );
        assert_eq!(
            idle_stop_override(" default "),
            Ok(IdleStopOverride::Setting)
        );
        assert_eq!(
            idle_stop_override("soon").unwrap_err(),
            "expected a duration such as 30m, or never, got 'soon'; or default to follow the setting"
        );
    }
}

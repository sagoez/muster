use std::{fmt, str::FromStr, time::Duration};

use super::{PositiveDuration, ValueParseError};

/// The word for an idle stop that never comes.
const NEVER_WORD: &str = "never";
/// The form an idle stop is written in.
const IDLE_STOP_FORM: &str = "a duration such as 30m, or never";

/// How long a project with no shell in it keeps running before it stops, or
/// never. Not serde: its text form, through [`FromStr`] and [`fmt::Display`],
/// is how it travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdleStop {
    /// Stops once the project has had no shell for this long.
    After(PositiveDuration),
    /// Keeps running however long the project has no shell.
    Never,
}

impl IdleStop {
    /// How long it waits before stopping, or `None` when it never does.
    pub fn after(&self) -> Option<Duration> {
        match self {
            Self::After(duration) => Some(duration.into_inner()),
            Self::Never => None,
        }
    }
}

impl FromStr for IdleStop {
    type Err = ValueParseError;

    /// Reads `never` or a humantime duration such as `30m`.
    ///
    /// # Errors
    /// Returns [`ValueParseError`] for other text or a zero duration.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let trimmed = raw.trim();
        if trimmed == NEVER_WORD {
            return Ok(Self::Never);
        }
        trimmed.parse().map(Self::After).map_err(|_| {
            ValueParseError::builder()
                .expected(IDLE_STOP_FORM)
                .found(raw)
                .build()
        })
    }
}

impl fmt::Display for IdleStop {
    /// Writes the duration in humantime, or `never`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::After(duration) => write!(formatter, "{duration}"),
            Self::Never => formatter.write_str(NEVER_WORD),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_stop_reads_durations_and_never_and_displays_back() {
        for (raw, after) in [
            ("30m", Some(Duration::from_secs(30 * 60))),
            ("1h 30m", Some(Duration::from_secs(90 * 60))),
            ("never", None),
        ] {
            let idle_stop: IdleStop = format!(" {raw} ").parse().unwrap();

            assert_eq!(idle_stop.after(), after, "{raw}");
            assert_eq!(idle_stop.to_string(), raw);
            assert_eq!(idle_stop.to_string().parse::<IdleStop>(), Ok(idle_stop));
        }
    }

    #[test]
    fn idle_stop_refuses_zero_and_garbage_naming_the_form() {
        for raw in ["0s", "soon", "", "Never"] {
            let error = raw.parse::<IdleStop>().unwrap_err();

            assert!(error.to_string().contains(IDLE_STOP_FORM), "{raw}: {error}");
        }
    }
}

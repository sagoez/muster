use std::{
    env,
    ffi::OsStr,
    io::{self, IsTerminal},
};

use getset::CopyGetters;
use typed_builder::TypedBuilder;

/// Set to anything, asks every program for output without color.
const NO_COLOR_ENV: &str = "NO_COLOR";
/// Names the terminal type.
const TERM_ENV: &str = "TERM";
/// The terminal type that draws nothing but plain text.
const DUMB_TERM: &str = "dumb";

/// What a command finds around it that decides how it presents itself.
#[derive(Debug, Clone, Copy, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct Surroundings {
    /// The command was asked for JSON.
    json: bool,
    /// Standard input is a terminal a person types on.
    stdin_terminal: bool,
    /// Standard output is a terminal.
    stdout_terminal: bool,
    /// Standard error, where prompts and frames draw, is a terminal.
    stderr_terminal: bool,
    /// `NO_COLOR` is set to a non-empty value.
    no_color: bool,
    /// `TERM` is `dumb`.
    dumb_terminal: bool,
}

impl Surroundings {
    /// What this process finds around it, for a command asked for JSON when
    /// `json` is set.
    pub fn current(json: bool) -> Self {
        Self::builder()
            .json(json)
            .stdin_terminal(io::stdin().is_terminal())
            .stdout_terminal(io::stdout().is_terminal())
            .stderr_terminal(io::stderr().is_terminal())
            .no_color(asks_for_no_color(env::var_os(NO_COLOR_ENV).as_deref()))
            .dumb_terminal(env::var_os(TERM_ENV).is_some_and(|term| term == DUMB_TERM))
            .build()
    }

    /// Whether a person sits at a terminal that can show prompts, frames and
    /// spinners.
    pub fn interactive(self) -> bool {
        !self.json
            && self.stdin_terminal
            && self.stdout_terminal
            && self.stderr_terminal
            && !self.no_color
            && !self.dumb_terminal
    }

    /// Whether what is printed on stdout may carry colour: it is a terminal
    /// that draws colour, no JSON was asked for, and `NO_COLOR` is not set.
    pub fn colour(self) -> bool {
        !self.json && self.stdout_terminal && !self.no_color && !self.dumb_terminal
    }
}

/// Whether `NO_COLOR` set to `value` asks for output without color: only a
/// non-empty value does, as no-color.org defines it.
fn asks_for_no_color(value: Option<&OsStr>) -> bool {
    value.is_some_and(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty `NO_COLOR` is the same as an unset one.
    #[test]
    fn only_a_non_empty_no_color_counts() {
        assert!(!asks_for_no_color(None));
        assert!(!asks_for_no_color(Some(OsStr::new(""))));
        assert!(asks_for_no_color(Some(OsStr::new("1"))));
    }

    /// Surroundings where everything favors the interactive layout.
    fn at_a_terminal() -> Surroundings {
        Surroundings::builder()
            .json(false)
            .stdin_terminal(true)
            .stdout_terminal(true)
            .stderr_terminal(true)
            .no_color(false)
            .dumb_terminal(false)
            .build()
    }

    /// Any one of JSON, a redirected stream, `NO_COLOR` or a dumb terminal
    /// falls back to plain lines.
    #[test]
    fn any_one_reason_falls_back_to_plain() {
        assert!(at_a_terminal().interactive());
        let terminal = at_a_terminal();
        let fallbacks = [
            Surroundings {
                json: true,
                ..terminal
            },
            Surroundings {
                stdin_terminal: false,
                ..terminal
            },
            Surroundings {
                stdout_terminal: false,
                ..terminal
            },
            Surroundings {
                stderr_terminal: false,
                ..terminal
            },
            Surroundings {
                no_color: true,
                ..terminal
            },
            Surroundings {
                dumb_terminal: true,
                ..terminal
            },
        ];
        for fallback in fallbacks {
            assert!(!fallback.interactive(), "{fallback:?}");
        }
    }

    /// Colour on stdout needs only stdout to be a terminal that draws it,
    /// with neither JSON nor `NO_COLOR` asked for.
    #[test]
    fn colour_needs_a_colour_terminal_on_stdout() {
        let terminal = at_a_terminal();
        assert!(terminal.colour());
        assert!(
            Surroundings {
                stdin_terminal: false,
                stderr_terminal: false,
                ..terminal
            }
            .colour()
        );
        let plain = [
            Surroundings {
                json: true,
                ..terminal
            },
            Surroundings {
                stdout_terminal: false,
                ..terminal
            },
            Surroundings {
                no_color: true,
                ..terminal
            },
            Surroundings {
                dumb_terminal: true,
                ..terminal
            },
        ];
        for surroundings in plain {
            assert!(!surroundings.colour(), "{surroundings:?}");
        }
    }
}

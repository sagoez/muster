use std::num::TryFromIntError;

use strum::ParseError;

use crate::domain::{
    coordination::{
        EntryPositionError, KvKeyError, LockNameError, LockNoteError, ScratchpadKeyError,
        TodoIdError, TodoTitleError,
    },
    pty::SignalNameError,
    value::{
        ByteSizeError, CommandLineError, DescriptionError, DetachKeyError, EnvNameError,
        EnvValueError, LogMaxSizeError, PositiveDurationError, ProcessNameError,
        ScrollbackLinesError, StopGraceError,
    },
};

/// Why an empty stored value is refused.
const EMPTY: &str = "it is empty";
/// Why a stored value past its length cap is refused.
const TOO_LONG: &str = "it is longer than muster allows";
/// Why a stored number below its floor is refused.
const TOO_SMALL: &str = "it is below the least muster allows";
/// Why a stored number above its ceiling is refused.
const TOO_LARGE: &str = "it is above the most muster allows";
/// Why a stored number that does not fit its type is refused.
const OUT_OF_RANGE: &str = "it is out of range";
/// Why a stored name of a fixed set is refused.
const UNKNOWN_VARIANT: &str = "it is not one muster knows";
/// Why stored text that could drive a terminal is refused.
const UNSAFE_TEXT: &str = "it holds a control or format character";
/// Why a stored lock name is refused.
const NOT_A_LOCK_NAME: &str = "it holds a character other than a-z, 0-9, '.', '_' or '-'";
/// Why a stored size of zero is refused.
const ZERO_SIZE: &str = "it is zero";
/// Why a stored log size outside its bounds is refused.
const LOG_SIZE_OUT_OF_BOUNDS: &str = "it is outside the log sizes muster allows";
/// Why a stored detach key is refused.
const NOT_A_CHORD: &str = "it is not a Control chord muster knows";
/// Why a stored duration is refused.
const NOT_POSITIVE_MILLIS: &str = "it is not a positive whole number of milliseconds";
/// Why a stored stop grace is refused.
const NOT_A_GRACE: &str =
    "it is not a positive whole number of milliseconds up to the longest grace";
/// Why a stored variable name is refused.
const NOT_AN_ENV_NAME: &str = "it is empty, holds '=' or NUL, or starts with MUSTER_";
/// Why a stored variable value is refused.
const HOLDS_NUL: &str = "it holds a NUL character";

/// A refusal of a stored value, read as the sentence that follows the field
/// it names.
pub(super) trait Refusal {
    /// The plain sentence saying why the value was refused.
    fn reason(&self) -> &'static str;
}

impl Refusal for TryFromIntError {
    /// A stored integer that does not fit is out of range.
    fn reason(&self) -> &'static str {
        OUT_OF_RANGE
    }
}

impl Refusal for ParseError {
    /// The reason a stored name of a fixed set is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::VariantNotFound => UNKNOWN_VARIANT,
        }
    }
}

impl Refusal for ProcessNameError {
    /// The reason a stored process name is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::NotEmptyViolated => EMPTY,
            Self::LenCharMaxViolated => TOO_LONG,
            Self::PredicateViolated => UNSAFE_TEXT,
        }
    }
}

impl Refusal for CommandLineError {
    /// The reason a stored command is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::NotEmptyViolated => EMPTY,
            Self::LenCharMaxViolated => TOO_LONG,
        }
    }
}

impl Refusal for DescriptionError {
    /// The reason a stored description is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::NotEmptyViolated => EMPTY,
            Self::LenCharMaxViolated => TOO_LONG,
        }
    }
}

impl Refusal for EnvNameError {
    /// The reason a stored variable name is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::LenCharMaxViolated => TOO_LONG,
            Self::PredicateViolated => NOT_AN_ENV_NAME,
        }
    }
}

impl Refusal for EnvValueError {
    /// The reason a stored variable value is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::LenCharMaxViolated => TOO_LONG,
            Self::PredicateViolated => HOLDS_NUL,
        }
    }
}

impl Refusal for ByteSizeError {
    /// The reason a stored size is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::PredicateViolated => ZERO_SIZE,
        }
    }
}

impl Refusal for LogMaxSizeError {
    /// The reason a stored log size is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::PredicateViolated => LOG_SIZE_OUT_OF_BOUNDS,
        }
    }
}

impl Refusal for ScrollbackLinesError {
    /// The reason a stored scrollback length is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::LessOrEqualViolated => TOO_LARGE,
        }
    }
}

impl Refusal for DetachKeyError {
    /// The reason a stored detach key is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::PredicateViolated => NOT_A_CHORD,
        }
    }
}

impl Refusal for PositiveDurationError {
    /// The reason a stored duration is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::PredicateViolated => NOT_POSITIVE_MILLIS,
        }
    }
}

impl Refusal for StopGraceError {
    /// The reason a stored stop grace is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::PredicateViolated => NOT_A_GRACE,
        }
    }
}

impl Refusal for SignalNameError {
    /// The reason a stored signal name is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::NotEmptyViolated => EMPTY,
        }
    }
}

impl Refusal for LockNameError {
    /// The reason a stored lock name is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::NotEmptyViolated => EMPTY,
            Self::LenCharMaxViolated => TOO_LONG,
            Self::PredicateViolated => NOT_A_LOCK_NAME,
        }
    }
}

impl Refusal for LockNoteError {
    /// The reason a stored lock note is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::NotEmptyViolated => EMPTY,
            Self::LenCharMaxViolated => TOO_LONG,
            Self::PredicateViolated => UNSAFE_TEXT,
        }
    }
}

impl Refusal for ScratchpadKeyError {
    /// The reason a stored note key is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::NotEmptyViolated => EMPTY,
            Self::LenCharMaxViolated => TOO_LONG,
        }
    }
}

impl Refusal for KvKeyError {
    /// The reason a stored key-value key is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::NotEmptyViolated => EMPTY,
            Self::LenCharMaxViolated => TOO_LONG,
        }
    }
}

impl Refusal for TodoIdError {
    /// The reason a stored todo id is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::NotEmptyViolated => EMPTY,
            Self::LenCharMaxViolated => TOO_LONG,
        }
    }
}

impl Refusal for TodoTitleError {
    /// The reason a stored todo title is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::NotEmptyViolated => EMPTY,
        }
    }
}

impl Refusal for EntryPositionError {
    /// The reason a stored entry position is refused.
    fn reason(&self) -> &'static str {
        match self {
            Self::GreaterOrEqualViolated => TOO_SMALL,
        }
    }
}

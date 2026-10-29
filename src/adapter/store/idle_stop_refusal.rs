/// Why stored idle-stop columns do not read as an idle stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IdleStopRefusal {
    /// The kind is unknown, or its milliseconds do not go with it.
    Mismatched,
    /// The milliseconds are not a positive whole number.
    NotPositive,
}

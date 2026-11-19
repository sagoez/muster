use strum::{AsRefStr, Display, EnumIter, EnumProperty, IntoEnumIterator};

/// The property marking a field that names what a condition waits for,
/// rather than modifying it.
const KIND: &str = "kind";

/// A field of a wait condition, as an agent writes it: a kind, of which a
/// condition names exactly one, or a modifier of one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display, AsRefStr, EnumIter, EnumProperty)]
#[strum(serialize_all = "kebab-case")]
pub(crate) enum ConditionField {
    /// A line of output.
    #[strum(props(kind = true))]
    Log,
    /// Reads log as a regex.
    Regex,
    /// A localhost port that accepts a connection.
    #[strum(props(kind = true))]
    Port,
    /// A localhost URL that answers.
    #[strum(props(kind = true))]
    Http,
    /// The HTTP status to wait for.
    Status,
    /// A lifecycle state.
    #[strum(props(kind = true))]
    State,
    /// The process's stored readiness check.
    #[strum(props(kind = true))]
    Ready,
}

impl ConditionField {
    /// Whether this field names what a condition waits for.
    pub(crate) fn is_kind(self) -> bool {
        self.get_bool(KIND).unwrap_or_default()
    }

    /// The fields that name what a condition waits for, in order.
    pub(crate) fn kinds() -> impl Iterator<Item = Self> {
        Self::iter().filter(|field| field.is_kind())
    }
}

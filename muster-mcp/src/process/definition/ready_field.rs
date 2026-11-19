use strum::{AsRefStr, Display, EnumIter, EnumProperty, IntoEnumIterator};

/// The property marking a field that names what a check probes, rather than
/// modifying it.
const PROBE: &str = "probe";

/// A field of a readiness check, as an agent writes it: a probe, of which a
/// check names exactly one, or what goes with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display, AsRefStr, EnumIter, EnumProperty)]
#[strum(serialize_all = "kebab-case")]
pub(crate) enum ReadyField {
    /// A line of output.
    #[strum(props(probe = true))]
    Log,
    /// Reads log as a regex.
    Regex,
    /// A localhost port that accepts a connection.
    #[strum(props(probe = true))]
    Port,
    /// A localhost URL that answers below 500.
    #[strum(props(probe = true))]
    Http,
    /// How long a run may take to become ready.
    Timeout,
}

impl ReadyField {
    /// Whether this field names what a check probes.
    pub(crate) fn is_probe(self) -> bool {
        self.get_bool(PROBE).unwrap_or_default()
    }

    /// The fields that name what a check probes, in order.
    pub(crate) fn probes() -> impl Iterator<Item = Self> {
        Self::iter().filter(|field| field.is_probe())
    }
}

use nutype::nutype;

/// When a hook load began, in nanoseconds since the Unix epoch, as precise
/// as its shell's clock reads.
#[nutype(derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Display,
    FromStr,
    Serialize,
    Deserialize
))]
pub struct LoadStart(u64);

use nutype::nutype;

/// Which loading of muster's hook in a shell made a report. A shell that
/// `exec`s itself keeps its pid and start token but loads the hook again,
/// numbering its prompts from one under a new load id.
#[nutype(derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Display,
    FromStr,
    Serialize,
    Deserialize
))]
pub struct LoadId(u64);

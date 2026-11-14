use nutype::nutype;

/// The number a shell's hook gives each prompt's report, rising, so a report
/// arriving after a newer one is told apart and dropped.
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
pub struct ReportSeq(u64);

use getset::CopyGetters;
use typed_builder::TypedBuilder;

use super::{LoadId, ReportSeq};

/// Where a report stands among its shell's reports: the hook load that made
/// it and the prompt it was made at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct ReportStamp {
    /// The hook load that made the report.
    load: LoadId,
    /// The prompt, counted within that load.
    seq: ReportSeq,
}

impl ReportStamp {
    /// Whether a report stamped `self` is newer than one stamped `earlier`:
    /// a later prompt of the same load, or any prompt of another load, which
    /// only a shell that loaded the hook again can make.
    pub fn supersedes(&self, earlier: &Self) -> bool {
        self.load != earlier.load || self.seq > earlier.seq
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::occupancy::testing::stamp;

    #[test]
    fn a_later_prompt_of_the_same_load_supersedes_and_an_earlier_one_does_not() {
        assert!(stamp(7, 5).supersedes(&stamp(7, 4)));
        assert!(!stamp(7, 5).supersedes(&stamp(7, 5)));
        assert!(!stamp(7, 4).supersedes(&stamp(7, 5)));
    }

    /// An `exec` of the shell loads the hook again, so its first prompt
    /// supersedes every prompt of the load before.
    #[test]
    fn any_prompt_of_another_load_supersedes() {
        assert!(stamp(8, 1).supersedes(&stamp(7, 500)));
        assert!(stamp(6, 1).supersedes(&stamp(7, 500)));
    }
}

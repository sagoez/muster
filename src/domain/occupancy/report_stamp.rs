use getset::CopyGetters;
use typed_builder::TypedBuilder;

use super::{LoadId, ReportSeq};

/// Where a report stands among its shell's reports: the hook load that made
/// it and the prompt it was made at. Stamps order by load, then by prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct ReportStamp {
    /// The hook load that made the report.
    load: LoadId,
    /// The prompt, counted within that load.
    seq: ReportSeq,
}

impl ReportStamp {
    /// Whether a report stamped `self` is newer than one stamped `earlier`:
    /// a later prompt of the same load, or any prompt of a later load, which
    /// only a shell that loaded the hook again can make.
    pub fn supersedes(&self, earlier: &Self) -> bool {
        self > earlier
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::occupancy::testing::{loaded_stamp, stamp};

    #[test]
    fn a_later_prompt_of_the_same_load_supersedes_and_an_earlier_one_does_not() {
        assert!(stamp(7, 5).supersedes(&stamp(7, 4)));
        assert!(!stamp(7, 5).supersedes(&stamp(7, 5)));
        assert!(!stamp(7, 4).supersedes(&stamp(7, 5)));
    }

    /// An `exec` of the shell loads the hook again, so its first prompt
    /// supersedes every prompt of the load before, while a late report of
    /// that older load supersedes nothing of the newer one.
    #[test]
    fn only_a_prompt_of_a_later_load_supersedes_another_load() {
        assert!(stamp(8, 1).supersedes(&stamp(7, 500)));
        assert!(!stamp(6, 500).supersedes(&stamp(7, 1)));
    }

    /// Two loads begun at the same clock reading order by the pid each
    /// spawned.
    #[test]
    fn the_load_s_pid_breaks_a_tie_of_its_start() {
        assert!(loaded_stamp(7, 300, 1).supersedes(&loaded_stamp(7, 200, 9)));
        assert!(!loaded_stamp(7, 200, 9).supersedes(&loaded_stamp(7, 300, 1)));
    }
}

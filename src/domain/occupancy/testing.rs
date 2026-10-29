//! Fixtures the occupancy tests across the crate share.

use super::{LoadId, ReportSeq, ReportStamp};

/// The stamp of prompt `seq` of hook load `load`.
pub(crate) fn stamp(load: u64, seq: u64) -> ReportStamp {
    ReportStamp::builder()
        .load(LoadId::new(load))
        .seq(ReportSeq::new(seq))
        .build()
}

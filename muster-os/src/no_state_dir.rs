use thiserror::Error;

/// muster finds no state directory on this platform, which every database,
/// socket and log lives under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("cannot find muster's state directory; set HOME or XDG_STATE_HOME")]
pub struct NoStateDir;

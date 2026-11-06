use strum::{Display, EnumIter};

/// One field of a process definition, as an edit names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Display, EnumIter)]
#[strum(serialize_all = "kebab-case")]
pub enum ProcessField {
    /// The shell command it runs.
    Command,
    /// Where it runs.
    WorkingDir,
    /// Its one-line description.
    Description,
    /// The variables it runs with.
    Environment,
    /// Whether `muster up` starts it.
    Autostart,
    /// What happens when its child exits.
    Restart,
    /// Its graceful stop signal.
    StopSignal,
    /// Its grace period before force-kill.
    StopGrace,
    /// Its readiness check.
    Ready,
    /// The ports it listens on.
    Ports,
}

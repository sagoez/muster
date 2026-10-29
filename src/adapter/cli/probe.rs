use super::running_service::RunningService;

/// What a client finds at the service socket.
pub(super) enum Probe {
    /// Nothing accepts there.
    Absent,
    /// A service is exiting and takes no more requests.
    Exiting,
    /// A service answers.
    Running(RunningService),
}

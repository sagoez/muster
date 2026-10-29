use super::running_service::RunningService;

/// How one attempt to start the service ended.
pub(super) enum ServiceStart {
    /// The service answers.
    Running(RunningService),
    /// The spawned service exited at once because another service holds the
    /// state directory.
    Lost,
}

use std::path::Path;

/// Driven port: watches a workspace config file for external changes (an edit,
/// or an append by the `muster` CLI) so the running app can pick them up.
pub trait ConfigWatcher {
    /// Watch `path`, replacing any previously watched path. Best-effort: an
    /// unwatchable path simply yields no change notifications.
    fn watch(&mut self, path: &Path);
}

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::value::{PaneId, ProcessName};

/// A desktop notification about one process, such as a crash.
#[derive(Clone, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Notification {
    /// Pane of the process the notification is about.
    pane: PaneId,
    /// The process, used as the notification's context.
    source: ProcessName,
    /// Optional summary line; delivery falls back to `source` when absent.
    #[builder(default)]
    title: Option<String>,
    /// Optional notification message.
    #[builder(default)]
    body: Option<String>,
}

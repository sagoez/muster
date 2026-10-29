use std::{path::PathBuf, sync::Arc};

use typed_builder::TypedBuilder;

use super::{holder_pool::HolderPool, project_ports::ProjectPorts};
use crate::domain::settings::Settings;

/// What the service runs on. No `Getters`: the service takes it apart once.
#[derive(TypedBuilder)]
pub struct ServiceDeps {
    /// The running binary's version, answered to a client's hello.
    pub(super) version: String,
    /// The running binary's path, answered to a client's hello so an older
    /// client can say which muster it found.
    pub(super) binary: PathBuf,
    /// The holders every project's processes run under.
    pub(super) holders: Arc<dyn HolderPool>,
    /// Builds each project's driven adapters as it is loaded.
    pub(super) ports: Box<dyn ProjectPorts>,
    /// The user's settings, read once at start.
    pub(super) settings: Settings,
}

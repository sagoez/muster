use std::{path::PathBuf, sync::Arc};

use getset::Getters;
use muster_client::ServiceAccess;
use muster_domain::{
    config::ConfigError,
    value::{LocalPort, ProjectName},
};
use muster_protocol::holder::HolderDirs;
use typed_builder::TypedBuilder;

use super::{manifest_reader::ManifestReader, opened_state::OpenedState};

/// Opens muster's state read-only, never creating or migrating it; called
/// again while it cannot be read, so a database created or migrated later
/// shows up.
pub type StateOpener = Arc<dyn Fn() -> Result<OpenedState, ConfigError> + Send + Sync>;

/// What `muster view` works with: where it listens, where it was started,
/// the floor it opens on, the service it reads, the state it reads, the
/// manifests it looks at and where each process's log lives.
#[derive(Clone, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ViewDeps {
    /// The port to listen on; a free one when none is given.
    port: Option<LocalPort>,
    /// The folder the view was started in, whose project it shows first.
    cwd: PathBuf,
    /// The project whose floor the page opens on, when one was named.
    floor: Option<ProjectName>,
    /// The user's home folder, if known.
    home: Option<PathBuf>,
    /// How the view reaches a running service, which it never starts.
    service: Arc<dyn ServiceAccess>,
    /// Opens the state read-only.
    state: StateOpener,
    /// The database file the state lives in, watched so a database deleted
    /// and made again is opened afresh.
    database: PathBuf,
    /// Reads the manifests each icon is picked from.
    manifests: Arc<dyn ManifestReader>,
    /// Where each process's log lives.
    holder_dirs: HolderDirs,
}

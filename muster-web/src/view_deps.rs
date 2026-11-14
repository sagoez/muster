use std::{path::PathBuf, sync::Arc};

use getset::Getters;
use muster_client::ServiceAccess;
use muster_domain::{config::ConfigError, port::LocalState, value::LocalPort};
use muster_protocol::holder::HolderDirs;
use typed_builder::TypedBuilder;

use super::manifest_reader::ManifestReader;

/// Opens muster's state read-only when it exists at the current schema;
/// called again while there is none, so a database created later shows up.
pub type StateOpener =
    Arc<dyn Fn() -> Result<Option<Arc<dyn LocalState>>, ConfigError> + Send + Sync>;

/// What `muster view` works with: where it listens, where it was started,
/// the service it reads, the state it reads, the manifests it looks at and
/// where each process's log lives.
#[derive(Clone, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ViewDeps {
    /// The port to listen on; a free one when none is given.
    port: Option<LocalPort>,
    /// The folder the view was started in, whose project it shows first.
    cwd: PathBuf,
    /// The user's home folder, if known.
    home: Option<PathBuf>,
    /// How the view reaches a running service, which it never starts.
    service: Arc<dyn ServiceAccess>,
    /// Opens the state read-only.
    state: StateOpener,
    /// Reads the manifests each icon is picked from.
    manifests: Arc<dyn ManifestReader>,
    /// Where each process's log lives.
    holder_dirs: HolderDirs,
}

use std::path::PathBuf;

use getset::Getters;
use muster_domain::value::{LocalPort, ProjectName};
use typed_builder::TypedBuilder;

/// What `muster view` asks the composition root to serve: the port to
/// listen on, whether to open a browser, the folder it was started in, and
/// the floor the page opens on.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ViewLaunch {
    /// The port to listen on; a free one when none is given.
    port: Option<LocalPort>,
    /// Whether to open the page in the default browser.
    open: bool,
    /// The folder the view was started in.
    cwd: PathBuf,
    /// The registered project `--project` named, whose floor the page opens
    /// on; none when it named none.
    floor: Option<ProjectName>,
}

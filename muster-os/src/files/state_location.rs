use std::{
    env,
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
};

use directories::{BaseDirs, ProjectDirs};
use getset::Getters;
use muster_domain::constants::APP_NAME;
use typed_builder::TypedBuilder;

use super::{no_state_dir::NoStateDir, private_dir::STATE_HOME_ENV};

/// Folder under the home folder the state directory goes in when
/// `XDG_STATE_HOME` names none: the XDG default, on every platform.
const DEFAULT_STATE_HOME: &str = ".local/state";

/// Where muster keeps its state on this machine, and where an earlier muster
/// kept it when that was somewhere else.
#[derive(Debug, Clone, PartialEq, Eq, Getters, TypedBuilder)]
pub struct StateLocation {
    /// muster's state directory: `muster` in `$XDG_STATE_HOME` when that is
    /// absolute, else `~/.local/state/muster`.
    #[getset(get = "pub")]
    dir: PathBuf,
    /// The state directory an earlier muster used on this platform
    /// (`~/Library/Application Support/muster` on macOS), kept only when it
    /// differs from `dir` and `dir` is the default one, the place it moves to.
    #[getset(get = "pub")]
    legacy: Option<PathBuf>,
}

impl StateLocation {
    /// This user's location, from `XDG_STATE_HOME`, the home folder and the
    /// place an earlier muster kept its state on this platform.
    ///
    /// # Errors
    /// Returns [`NoStateDir`] when `XDG_STATE_HOME` is not an absolute path
    /// and no home folder can be found.
    pub fn current() -> Result<Self, NoStateDir> {
        let home = BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
        Self::resolve(
            env::var_os(STATE_HOME_ENV).as_deref(),
            home.as_deref(),
            earlier_state_dir().as_deref(),
        )
    }

    /// The location that `state_home`, the value of `XDG_STATE_HOME`, and
    /// `home` give, with `earlier` as its legacy directory when the state
    /// directory is the default one and is not `earlier` itself. A relative
    /// `state_home` is ignored, as XDG says.
    ///
    /// # Errors
    /// Returns [`NoStateDir`] when there is neither an absolute `state_home`
    /// nor a `home`.
    pub fn resolve(
        state_home: Option<&OsStr>,
        home: Option<&Path>,
        earlier: Option<&Path>,
    ) -> Result<Self, NoStateDir> {
        let chosen = state_home.map(Path::new).filter(|root| root.is_absolute());
        let dir = match (chosen, home) {
            (Some(root), _) => root.join(APP_NAME),
            (None, Some(home)) => home.join(DEFAULT_STATE_HOME).join(APP_NAME),
            (None, None) => return Err(NoStateDir),
        };
        let legacy = earlier
            .filter(|earlier| chosen.is_none() && *earlier != dir)
            .map(Path::to_path_buf);
        Ok(Self::builder().dir(dir).legacy(legacy).build())
    }

    /// The legacy directory while it still waits to move: a real directory,
    /// not a link, and the state directory does not exist yet.
    pub fn pending_move(&self) -> Option<&Path> {
        self.legacy
            .as_deref()
            .filter(|legacy| is_real_dir(legacy) && is_absent(&self.dir))
    }

    /// The directory muster uses: the legacy one while it waits to move, so
    /// nothing kept there is passed over before it has, else the state
    /// directory.
    pub fn in_use(&self) -> &Path {
        self.pending_move().unwrap_or(&self.dir)
    }
}

/// Where muster kept its state before it followed XDG on every platform:
/// the platform's state location, or its local data where it has none, as
/// on macOS.
fn earlier_state_dir() -> Option<PathBuf> {
    ProjectDirs::from("", "", APP_NAME).map(|dirs| {
        dirs.state_dir()
            .unwrap_or_else(|| dirs.data_local_dir())
            .to_path_buf()
    })
}

/// Whether `path` is a directory itself, not a link to one.
fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

/// Whether nothing, not even a dangling link, is at `path`.
fn is_absent(path: &Path) -> bool {
    matches!(fs::symlink_metadata(path), Err(error) if error.kind() == io::ErrorKind::NotFound)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::*;

    /// A home folder as macOS names one.
    const MAC_HOME: &str = "/Users/sam";
    /// Where an earlier muster kept its state under that home on macOS.
    const MAC_LEGACY: &str = "/Users/sam/Library/Application Support/muster";
    /// A home folder as Linux names one.
    const LINUX_HOME: &str = "/home/sam";
    /// An absolute `XDG_STATE_HOME`.
    const STATE_HOME: &str = "/var/state";
    /// A relative `XDG_STATE_HOME`, which XDG says to ignore.
    const RELATIVE_STATE_HOME: &str = "state";

    /// The location under `home` with no `XDG_STATE_HOME` and an earlier
    /// directory `earlier`.
    fn unset(home: &Path, earlier: &Path) -> StateLocation {
        StateLocation::resolve(None, Some(home), Some(earlier)).unwrap()
    }

    /// With no `XDG_STATE_HOME`, the state directory is `~/.local/state/muster`
    /// on macOS as on Linux, and macOS's old place is the legacy one to move.
    #[test]
    fn unset_state_home_is_the_xdg_default_on_every_platform() {
        let mac = unset(Path::new(MAC_HOME), Path::new(MAC_LEGACY));
        assert_eq!(mac.dir(), Path::new("/Users/sam/.local/state/muster"));
        assert_eq!(mac.legacy().as_deref(), Some(Path::new(MAC_LEGACY)));

        let linux_default = Path::new("/home/sam/.local/state/muster");
        let linux = unset(Path::new(LINUX_HOME), linux_default);
        assert_eq!(linux.dir(), linux_default);
        assert_eq!(linux.legacy(), &None, "Linux has nothing to move");
    }

    /// An absolute `XDG_STATE_HOME` holds the state directory, and no legacy
    /// directory is moved into a place the user chose.
    #[test]
    fn an_absolute_state_home_holds_the_state_dir() {
        let location = StateLocation::resolve(
            Some(OsStr::new(STATE_HOME)),
            Some(Path::new(MAC_HOME)),
            Some(Path::new(MAC_LEGACY)),
        )
        .unwrap();

        assert_eq!(location.dir(), Path::new("/var/state/muster"));
        assert_eq!(location.legacy(), &None);
    }

    /// A relative `XDG_STATE_HOME` is ignored, so the default holds the
    /// state directory, and without a home there is none.
    #[test]
    fn a_relative_state_home_is_ignored() {
        let relative = Some(OsStr::new(RELATIVE_STATE_HOME));
        let location = StateLocation::resolve(
            relative,
            Some(Path::new(MAC_HOME)),
            Some(Path::new(MAC_LEGACY)),
        )
        .unwrap();

        assert_eq!(location, unset(Path::new(MAC_HOME), Path::new(MAC_LEGACY)));
        assert_eq!(
            StateLocation::resolve(relative, None, None),
            Err(NoStateDir)
        );
        assert_eq!(StateLocation::resolve(None, None, None), Err(NoStateDir));
    }

    /// The legacy directory is in use only while it is a real directory and
    /// the state directory does not exist; a link left in its place, or a
    /// state directory that exists, ends that.
    #[test]
    fn the_legacy_dir_is_in_use_only_until_the_state_dir_exists() {
        let scratch = TempDir::new().unwrap();
        let home = scratch.path();
        let legacy = home.join("Library/Application Support/muster");
        let location = unset(home, &legacy);
        assert_eq!(location.pending_move(), None, "nothing to move yet");
        assert_eq!(location.in_use(), location.dir());

        fs::create_dir_all(&legacy).unwrap();
        assert_eq!(location.pending_move(), Some(legacy.as_path()));
        assert_eq!(location.in_use(), legacy);

        fs::create_dir_all(location.dir()).unwrap();
        assert_eq!(location.pending_move(), None, "the state directory exists");
        assert_eq!(location.in_use(), location.dir());

        fs::remove_dir(location.dir()).unwrap();
        fs::remove_dir(&legacy).unwrap();
        symlink(location.dir(), &legacy).unwrap();
        assert_eq!(location.pending_move(), None, "a link is not moved");
    }
}

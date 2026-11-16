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

use super::no_state_dir::NoStateDir;

/// Variable naming the state directory's parent; set it to a shorter folder
/// of your own when a holder's socket path does not fit.
pub const STATE_HOME_ENV: &str = "XDG_STATE_HOME";
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
    /// The state directory an earlier muster used on this platform whatever
    /// `XDG_STATE_HOME` said (`~/Library/Application Support/muster` on
    /// macOS), kept only when it differs from `dir`, the place it moves to.
    #[getset(get = "pub")]
    legacy: Option<PathBuf>,
}

impl StateLocation {
    /// This user's location, from `XDG_STATE_HOME`, the home folder and the
    /// place an earlier muster kept its state on this platform, which is
    /// found under the home folder.
    ///
    /// # Errors
    /// Returns [`NoStateDir`] when `XDG_STATE_HOME` is not an absolute path
    /// and no absolute home folder can be found.
    pub fn current() -> Result<Self, NoStateDir> {
        let home = BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
        Self::resolve(
            env::var_os(STATE_HOME_ENV).as_deref(),
            home.as_deref(),
            earlier_state_dir().as_deref(),
        )
    }

    /// The location that `state_home`, the value of `XDG_STATE_HOME`, and
    /// `home` give, with `earlier` as its legacy directory unless it is the
    /// state directory itself. A relative `state_home` is ignored, as XDG
    /// says, and so is a relative `home`, which would make the state
    /// directory follow the working directory.
    ///
    /// # Errors
    /// Returns [`NoStateDir`] when there is neither an absolute `state_home`
    /// nor an absolute `home`.
    pub fn resolve(
        state_home: Option<&OsStr>,
        home: Option<&Path>,
        earlier: Option<&Path>,
    ) -> Result<Self, NoStateDir> {
        let chosen = state_home.map(Path::new).filter(|root| root.is_absolute());
        let home = home.filter(|home| home.is_absolute());
        let dir = match (chosen, home) {
            (Some(root), _) => root.join(APP_NAME),
            (None, Some(home)) => home.join(DEFAULT_STATE_HOME).join(APP_NAME),
            (None, None) => return Err(NoStateDir),
        };
        let legacy = earlier
            .filter(|earlier| earlier.is_absolute() && *earlier != dir)
            .map(Path::to_path_buf);
        Ok(Self::builder().dir(dir).legacy(legacy).build())
    }

    /// The legacy directory while it still waits to move: a real directory,
    /// not a link, and nothing, not even a dangling link, is at the state
    /// directory yet.
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

    /// The legacy directory when it will never move but still leads to a
    /// folder of its own: something is already at the state directory, or
    /// it is a link to some other folder. The link a move leaves behind
    /// leads to the state directory, so it is not left behind.
    pub fn left_behind(&self) -> Option<&Path> {
        self.legacy.as_deref().filter(|legacy| {
            fs::metadata(legacy).is_ok_and(|metadata| metadata.is_dir())
                && self.pending_move().is_none()
                && !same_dir(legacy, &self.dir)
        })
    }
}

/// Where muster kept its state before it followed XDG on every platform:
/// the platform's state location, or its local data where it has none, as
/// on macOS, which never looked at `XDG_STATE_HOME`.
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
pub fn is_absent(path: &Path) -> bool {
    matches!(fs::symlink_metadata(path), Err(error) if error.kind() == io::ErrorKind::NotFound)
}

/// Whether `a` and `b` both lead to one existing directory.
fn same_dir(a: &Path, b: &Path) -> bool {
    matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
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
    /// A relative home folder.
    const RELATIVE_HOME: &str = "sam";
    /// Where an earlier muster kept its state under a scratch home.
    const SCRATCH_LEGACY: &str = "Library/Application Support/muster";
    /// A folder a legacy link may lead to instead of the state directory.
    const ELSEWHERE: &str = "elsewhere";

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

    /// An absolute `XDG_STATE_HOME` holds the state directory, and macOS's
    /// old place, which an earlier muster used whatever it said, still moves
    /// there.
    #[test]
    fn an_absolute_state_home_holds_the_state_dir_and_takes_the_legacy_one() {
        let location = StateLocation::resolve(
            Some(OsStr::new(STATE_HOME)),
            Some(Path::new(MAC_HOME)),
            Some(Path::new(MAC_LEGACY)),
        )
        .unwrap();

        assert_eq!(location.dir(), Path::new("/var/state/muster"));
        assert_eq!(location.legacy().as_deref(), Some(Path::new(MAC_LEGACY)));
    }

    /// A relative `XDG_STATE_HOME` is ignored, so the default holds the
    /// state directory; a relative home is no home, and without one there
    /// is no state directory.
    #[test]
    fn a_relative_state_home_or_home_is_ignored() {
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
        assert_eq!(
            StateLocation::resolve(None, Some(Path::new(RELATIVE_HOME)), None),
            Err(NoStateDir)
        );
    }

    /// The legacy directory is in use only while it is a real directory and
    /// nothing is at the state directory; a link left in its place, or a
    /// state directory that exists, ends that.
    #[test]
    fn the_legacy_dir_is_in_use_only_until_the_state_dir_exists() {
        let scratch = TempDir::new().unwrap();
        let home = scratch.path();
        let legacy = home.join(SCRATCH_LEGACY);
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

    /// A legacy directory that will never move is left behind when the
    /// state directory already exists, when something else is in its way,
    /// or when it is a link to another folder; the link a move leaves
    /// behind is not.
    #[test]
    fn a_legacy_dir_that_cannot_move_is_left_behind() {
        let scratch = TempDir::new().unwrap();
        let home = scratch.path();
        let legacy = home.join(SCRATCH_LEGACY);
        let location = unset(home, &legacy);
        fs::create_dir_all(&legacy).unwrap();
        assert_eq!(location.left_behind(), None, "it is waiting to move");

        fs::create_dir_all(location.dir()).unwrap();
        assert_eq!(location.left_behind(), Some(legacy.as_path()));

        fs::remove_dir(location.dir()).unwrap();
        fs::write(location.dir(), b"").unwrap();
        assert_eq!(location.left_behind(), Some(legacy.as_path()), "a file");
        fs::remove_file(location.dir()).unwrap();

        fs::create_dir_all(location.dir()).unwrap();
        fs::remove_dir(&legacy).unwrap();
        symlink(location.dir(), &legacy).unwrap();
        assert_eq!(location.left_behind(), None, "the moved link");

        let elsewhere = home.join(ELSEWHERE);
        fs::create_dir(&elsewhere).unwrap();
        fs::remove_file(&legacy).unwrap();
        symlink(&elsewhere, &legacy).unwrap();
        assert_eq!(location.left_behind(), Some(legacy.as_path()), "a link");
    }
}

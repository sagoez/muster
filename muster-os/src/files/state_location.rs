use std::{
    env,
    ffi::OsStr,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use directories::{BaseDirs, ProjectDirs};
use getset::Getters;
use muster_domain::constants::APP_NAME;
use rustix::process::geteuid;
use typed_builder::TypedBuilder;

use super::{
    no_state_dir::NoStateDir,
    path::{is_absent, is_real_dir},
};

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

    /// Where the legacy place leads, when it is a link this user made to an
    /// existing directory and nothing is at the state directory. The link a
    /// move leaves is the one record of where this user's state went, so a
    /// process that resolves another `XDG_STATE_HOME` than the one that
    /// moved it, as an app started outside the shell does, uses that state
    /// rather than making a second, empty one.
    pub fn linked_state(&self) -> Option<PathBuf> {
        let legacy = self.legacy.as_deref()?;
        let ours = fs::symlink_metadata(legacy)
            .is_ok_and(|link| link.is_symlink() && link.uid() == geteuid().as_raw());
        if !ours || !is_absent(&self.dir) {
            return None;
        }
        fs::canonicalize(legacy)
            .ok()
            .filter(|target| target.is_dir())
    }

    /// The directory muster uses: the legacy one while it waits to move, so
    /// nothing kept there is passed over before it has; where its link
    /// leads, as [`Self::linked_state`] says; else the state directory.
    pub fn in_use(&self) -> PathBuf {
        if let Some(legacy) = self.pending_move() {
            return legacy.to_path_buf();
        }
        self.linked_state().unwrap_or_else(|| self.dir.clone())
    }

    /// The folder the legacy place still leads to, canonical, when it will
    /// never move and is not the state directory in use: something is
    /// already at the state directory, or it is a link to some other
    /// folder.
    pub fn left_behind(&self) -> Option<PathBuf> {
        let legacy = self.legacy.as_deref()?;
        if self.pending_move().is_some() {
            return None;
        }
        let found = fs::canonicalize(legacy)
            .ok()
            .filter(|found| found.is_dir())?;
        let in_use = fs::canonicalize(self.in_use()).ok();
        (in_use.as_ref() != Some(&found)).then_some(found)
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

    /// The default state directory under `home`.
    fn default_dir(home: &str) -> PathBuf {
        Path::new(home).join(DEFAULT_STATE_HOME).join(APP_NAME)
    }

    /// With no `XDG_STATE_HOME`, the state directory is `~/.local/state/muster`
    /// on macOS as on Linux, and macOS's old place is the legacy one to move.
    #[test]
    fn unset_state_home_is_the_xdg_default_on_every_platform() {
        let mac = unset(Path::new(MAC_HOME), Path::new(MAC_LEGACY));
        assert_eq!(mac.dir(), &default_dir(MAC_HOME));
        assert_eq!(mac.legacy().as_deref(), Some(Path::new(MAC_LEGACY)));

        let linux_default = default_dir(LINUX_HOME);
        let linux = unset(Path::new(LINUX_HOME), &linux_default);
        assert_eq!(linux.dir(), &linux_default);
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

        assert_eq!(location.dir(), &Path::new(STATE_HOME).join(APP_NAME));
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
        assert_eq!(&location.in_use(), location.dir());

        fs::create_dir_all(&legacy).unwrap();
        assert_eq!(location.pending_move(), Some(legacy.as_path()));
        assert_eq!(location.in_use(), legacy);

        fs::create_dir_all(location.dir()).unwrap();
        assert_eq!(location.pending_move(), None, "the state directory exists");
        assert_eq!(&location.in_use(), location.dir());

        fs::remove_dir(location.dir()).unwrap();
        fs::remove_dir(&legacy).unwrap();
        symlink(location.dir(), &legacy).unwrap();
        assert_eq!(location.pending_move(), None, "a link is not moved");
    }

    /// With nothing at the state directory, the link this user's move left
    /// leads to where the state is, so a process that resolves another
    /// `XDG_STATE_HOME` uses that state instead of making a second; nothing
    /// is left behind then.
    #[test]
    fn the_legacy_link_anchors_the_state_another_state_home_moved() {
        let scratch = TempDir::new().unwrap();
        let home = scratch.path();
        let legacy = home.join(SCRATCH_LEGACY);
        let moved = home.join(ELSEWHERE);
        fs::create_dir(&moved).unwrap();
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        symlink(&moved, &legacy).unwrap();
        let location = unset(home, &legacy);

        assert_eq!(location.in_use(), moved.canonicalize().unwrap());
        assert_eq!(location.left_behind(), None);

        fs::create_dir_all(location.dir()).unwrap();
        assert_eq!(&location.in_use(), location.dir(), "an existing dir wins");
        assert_eq!(location.left_behind(), Some(moved.canonicalize().unwrap()));
    }

    /// A legacy directory that will never move is left behind, named where
    /// it really is, when the state directory already exists or something
    /// else is in its way; the link a move leaves is not.
    #[test]
    fn a_legacy_dir_that_cannot_move_is_left_behind() {
        let scratch = TempDir::new().unwrap();
        let home = scratch.path();
        let legacy = home.join(SCRATCH_LEGACY);
        let location = unset(home, &legacy);
        fs::create_dir_all(&legacy).unwrap();
        let found = legacy.canonicalize().unwrap();
        assert_eq!(location.left_behind(), None, "it is waiting to move");

        fs::create_dir_all(location.dir()).unwrap();
        assert_eq!(location.left_behind(), Some(found.clone()));

        fs::remove_dir(location.dir()).unwrap();
        fs::write(location.dir(), b"").unwrap();
        assert_eq!(location.left_behind(), Some(found), "a file");
        fs::remove_file(location.dir()).unwrap();

        fs::create_dir_all(location.dir()).unwrap();
        fs::remove_dir(&legacy).unwrap();
        symlink(location.dir(), &legacy).unwrap();
        assert_eq!(location.left_behind(), None, "the moved link");
    }
}

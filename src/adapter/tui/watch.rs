use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use crossbeam_channel::Sender;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use super::event::RuntimeEvent;
use crate::{adapter::path, domain::port::ConfigWatcher};

/// A [`ConfigWatcher`] backed by `notify`. It watches the target file's parent
/// directory (non-recursively) rather than the file itself, so an atomic
/// temp-then-rename write, like the one the registry performs, is still seen;
/// events are filtered to the target file name and forwarded as
/// [`RuntimeEvent::ConfigChanged`].
pub struct NotifyConfigWatcher {
    events: Sender<RuntimeEvent>,
    watcher: Option<RecommendedWatcher>,
    target: Option<PathBuf>,
}

impl NotifyConfigWatcher {
    /// Creates a watcher that forwards change events on `events`.
    pub fn new(events: Sender<RuntimeEvent>) -> Self {
        Self {
            events,
            watcher: None,
            target: None,
        }
    }
}

impl ConfigWatcher for NotifyConfigWatcher {
    fn watch(&mut self, path: &Path) {
        let target = path::normalize(path);
        if self.target.as_deref() == Some(target.as_path()) {
            return;
        }
        let (Some(dir), Some(name)) = (
            target.parent().map(Path::to_path_buf),
            target.file_name().map(OsString::from),
        ) else {
            return;
        };

        // Dropping the old watcher unwatches the previous directory. Clear the
        // target too, so that if setting up the replacement below fails, a later
        // re-watch of this same path is not skipped by the early return above
        // while no watcher is actually live.
        self.watcher = None;
        self.target = None;
        let events = self.events.clone();
        let notify_path = target.clone();
        let handler = move |result: notify::Result<notify::Event>| {
            let Ok(event) = result else {
                return;
            };
            if event.kind.is_access() {
                return;
            }
            if event
                .paths
                .iter()
                .any(|changed| changed.file_name() == Some(name.as_os_str()))
            {
                let _ = events.send(RuntimeEvent::ConfigChanged {
                    path: notify_path.clone(),
                });
            }
        };

        let Ok(mut watcher) = notify::recommended_watcher(handler) else {
            return;
        };
        if watcher.watch(&dir, RecursiveMode::NonRecursive).is_ok() {
            self.watcher = Some(watcher);
            self.target = Some(target);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, time::Duration};

    use crossbeam_channel::bounded;

    use super::*;

    #[test]
    fn a_change_to_the_watched_file_sends_an_event() {
        let dir = std::env::temp_dir().join(format!("muster-watch-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("muster.yml");
        fs::write(&file, "commands: []\n").unwrap();

        let (sender, receiver) = bounded(16);
        let mut watcher = NotifyConfigWatcher::new(sender);
        watcher.watch(&file);

        fs::write(&file, "commands: [a]\n").unwrap();

        let event = receiver.recv_timeout(Duration::from_secs(5));
        assert!(
            matches!(event, Ok(RuntimeEvent::ConfigChanged { .. })),
            "a write to the watched file is reported"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_change_through_a_symlinked_config_is_seen_in_its_real_directory() {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!("muster-watch-link-{}", std::process::id()));
        let real_dir = base.join("real");
        let link_dir = base.join("link");
        fs::create_dir_all(&real_dir).unwrap();
        fs::create_dir_all(&link_dir).unwrap();
        let target = real_dir.join("muster.yml");
        fs::write(&target, "commands: []\n").unwrap();
        let link = link_dir.join("muster.yml");
        symlink(&target, &link).unwrap();

        let (sender, receiver) = bounded(16);
        let mut watcher = NotifyConfigWatcher::new(sender);
        watcher.watch(&link); // watch via the symlink, in a different directory

        // As save_workspace does: canonicalize, then atomic temp+rename in the
        // real target's own directory.
        let temp = real_dir.join("muster.yml.tmp");
        fs::write(&temp, "commands: [a]\n").unwrap();
        fs::rename(&temp, &target).unwrap();

        let event = receiver.recv_timeout(Duration::from_secs(5));
        assert!(
            matches!(event, Ok(RuntimeEvent::ConfigChanged { .. })),
            "a save to the symlink's resolved target is reported"
        );

        let _ = fs::remove_dir_all(&base);
    }
}

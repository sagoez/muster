use std::{
    collections::VecDeque,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use super::{notice::Notice, shared_notices::SharedNotices};
use crate::domain::value::ProjectRoot;

/// Most notices kept in memory; the oldest is dropped to make room.
pub const MAX_NOTICES: usize = 256;

/// A bounded ring of the most recent notices across every project, each also
/// appended as a JSON line to an optional log file.
pub struct Notices {
    /// The most recent notices, oldest first.
    ring: VecDeque<Notice>,
    /// How many notices were ever pushed, so a reader can tell new ones apart.
    pushed: u64,
    /// File each notice is mirrored to, when there is one.
    log: Option<PathBuf>,
}

impl Notices {
    /// An empty ring mirroring to `log`, when given.
    pub fn new(log: Option<PathBuf>) -> Self {
        Self {
            ring: VecDeque::with_capacity(MAX_NOTICES),
            pushed: 0,
            log,
        }
    }

    /// An empty ring mirroring to `log`, ready to share.
    pub fn shared(log: Option<PathBuf>) -> SharedNotices {
        Arc::new(Mutex::new(Self::new(log)))
    }

    /// Records `notice`, made printable, dropping the oldest once the ring is
    /// full, and mirrors it to the log, best effort: a failed write never
    /// loses the notice itself.
    pub fn push(&mut self, notice: Notice) {
        let notice = notice.printable();
        mirror(self.log.as_deref(), &notice);
        if self.ring.len() == MAX_NOTICES {
            self.ring.pop_front();
        }
        self.ring.push_back(notice);
        self.pushed += 1;
    }

    /// The held notices for `project` and for the service as a whole, oldest
    /// first.
    pub fn recent(&self, project: &ProjectRoot) -> Vec<Notice> {
        self.ring
            .iter()
            .filter(|notice| notice.project().as_ref().is_none_or(|root| root == project))
            .cloned()
            .collect()
    }

    /// How many notices were ever pushed.
    pub fn pushed(&self) -> u64 {
        self.pushed
    }
}

/// Appends `notice` to `log`, when there is one, as one JSON line.
fn mirror(log: Option<&Path>, notice: &Notice) {
    let Some(log) = log else {
        return;
    };
    let Ok(mut line) = serde_json::to_vec(notice) else {
        return;
    };
    line.push(b'\n');
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log) {
        let _ = file.write_all(&line);
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use serde_json::Value;
    use tempfile::TempDir;

    use super::*;
    use crate::adapter::service::NoticeLevel;

    /// The project at `path`.
    fn root(path: &str) -> ProjectRoot {
        ProjectRoot::try_new(PathBuf::from(path)).unwrap()
    }

    /// An error notice for `project` saying `message`.
    fn notice(project: &ProjectRoot, message: &str) -> Notice {
        Notice::builder()
            .at(SystemTime::now())
            .project(Some(project.clone()))
            .process(None)
            .level(NoticeLevel::Error)
            .message(message.to_string())
            .build()
    }

    #[test]
    fn the_ring_keeps_only_the_newest_notices() {
        let project = root("/a");
        let mut notices = Notices::new(None);

        for index in 0..=MAX_NOTICES {
            notices.push(notice(&project, &index.to_string()));
        }

        let recent = notices.recent(&project);
        assert_eq!(recent.len(), MAX_NOTICES);
        assert_eq!(recent[0].message(), "1");
        assert_eq!(notices.pushed(), (MAX_NOTICES + 1) as u64);
    }

    #[test]
    fn recent_returns_only_the_projects_own_notices() {
        let first = root("/a");
        let second = root("/b");
        let mut notices = Notices::new(None);
        notices.push(notice(&first, "one"));
        notices.push(notice(&second, "two"));

        let recent = notices.recent(&second);

        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].message(), "two");
    }

    /// A notice about the service as a whole concerns every project, so each
    /// project's reader sees it.
    #[test]
    fn a_service_wide_notice_reaches_every_project() {
        let first = root("/a");
        let second = root("/b");
        let mut notices = Notices::new(None);
        notices.push(notice(&first, "one"));
        notices.push(
            Notice::builder()
                .at(SystemTime::now())
                .project(None)
                .process(None)
                .level(NoticeLevel::Warning)
                .message("service".to_string())
                .build(),
        );

        assert_eq!(notices.recent(&first).len(), 2);
        let second_sees: Vec<String> = notices
            .recent(&second)
            .iter()
            .map(|notice| notice.message().clone())
            .collect();
        assert_eq!(second_sees, vec!["service".to_string()]);
    }

    /// A message carrying terminal control sequences, as a process's error
    /// text can, is stored and logged without them.
    #[test]
    fn a_stored_notice_carries_no_control_characters() {
        let scratch = TempDir::new().unwrap();
        let log = scratch.path().join("service.log");
        let project = root("/a");
        let mut notices = Notices::new(Some(log.clone()));

        notices.push(notice(&project, "crashed: \x1b]0;pwned\x07\nexit 1"));

        let stored = notices.recent(&project)[0].message().clone();
        assert!(!stored.chars().any(char::is_control), "{stored:?}");
        assert!(stored.contains("exit 1"));
        let written = fs::read_to_string(log).unwrap();
        assert!(!written.contains('\x1b'));
    }

    #[test]
    fn each_notice_is_mirrored_as_a_json_line() {
        let scratch = TempDir::new().unwrap();
        let log = scratch.path().join("service.log");
        let project = root("/a");
        let mut notices = Notices::new(Some(log.clone()));

        notices.push(notice(&project, "one"));
        notices.push(notice(&project, "two"));

        let written = fs::read_to_string(log).unwrap();
        let lines: Vec<Value> = written
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1]["message"], "two");
        assert_eq!(lines[1]["level"], "error");
        assert_eq!(lines[1]["project"], "/a");
    }
}

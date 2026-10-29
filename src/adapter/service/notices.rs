use std::{
    collections::VecDeque,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use super::{notice::Notice, notice_level::NoticeLevel, project_key::ProjectKey};

/// Most warning and error notices kept in memory; the oldest is dropped to make
/// room.
pub const MAX_NOTICES: usize = 256;
/// Most attention notices kept in memory, held apart so a chatty process's
/// bells never push out a failure.
pub const MAX_ATTENTION_NOTICES: usize = 64;

/// Notices shared between the supervisors that raise them and the readers that
/// show them.
pub type SharedNotices = Arc<Mutex<Notices>>;

/// Bounded rings of the most recent notices across every project. Warnings and
/// errors are also appended as JSON lines to an optional log file; attention
/// is transient and stays in memory.
pub struct Notices {
    /// The most recent warnings and errors with their push order, oldest first.
    ring: VecDeque<(u64, Notice)>,
    /// The most recent attention notices with their push order, oldest first.
    attention: VecDeque<(u64, Notice)>,
    /// How many notices were ever pushed, so a reader can tell new ones apart.
    pushed: u64,
    /// File each warning and error is mirrored to, when there is one.
    log: Option<PathBuf>,
}

impl Notices {
    /// Empty rings mirroring to `log`, when given.
    pub fn new(log: Option<PathBuf>) -> Self {
        Self {
            ring: VecDeque::with_capacity(MAX_NOTICES),
            attention: VecDeque::with_capacity(MAX_ATTENTION_NOTICES),
            pushed: 0,
            log,
        }
    }

    /// Empty rings mirroring to `log`, ready to share.
    pub fn shared(log: Option<PathBuf>) -> SharedNotices {
        Arc::new(Mutex::new(Self::new(log)))
    }

    /// Records `notice` in its level's ring, dropping that ring's oldest once
    /// it is full. Only warnings and errors are mirrored to the log, best
    /// effort: a failed write never loses the notice itself.
    pub fn push(&mut self, notice: Notice) {
        let (ring, cap) = if *notice.level() == NoticeLevel::Attention {
            (&mut self.attention, MAX_ATTENTION_NOTICES)
        } else {
            mirror(self.log.as_deref(), &notice);
            (&mut self.ring, MAX_NOTICES)
        };
        if ring.len() == cap {
            ring.pop_front();
        }
        ring.push_back((self.pushed, notice));
        self.pushed += 1;
    }

    /// The held notices for `project` and for the service as a whole, of
    /// every level, oldest first.
    pub fn recent(&self, project: &ProjectKey) -> Vec<Notice> {
        let mut held: Vec<&(u64, Notice)> = self
            .ring
            .iter()
            .chain(&self.attention)
            .filter(|(_, notice)| notice.project().as_ref().is_none_or(|key| key == project))
            .collect();
        held.sort_by_key(|(order, _)| *order);
        held.into_iter().map(|(_, notice)| notice.clone()).collect()
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

    use tempfile::TempDir;

    use super::*;

    /// An error notice for `project` saying `message`.
    fn notice(project: &ProjectKey, message: &str) -> Notice {
        notice_at(NoticeLevel::Error, project, message)
    }

    /// A notice of `level` for `project` saying `message`.
    fn notice_at(level: NoticeLevel, project: &ProjectKey, message: &str) -> Notice {
        Notice::builder()
            .at(SystemTime::now())
            .project(Some(project.clone()))
            .process(None)
            .level(level)
            .message(message.to_string())
            .build()
    }

    /// Attention fills its own ring, so any number of bells leaves an older
    /// failure in place, and the levels still read back in push order.
    #[test]
    fn attention_never_evicts_a_failure() {
        let project = ProjectKey::new(PathBuf::from("/a/muster.yml"));
        let mut notices = Notices::new(None);
        notices.push(notice_at(NoticeLevel::Attention, &project, "bell"));
        notices.push(notice(&project, "failure"));

        for index in 0..MAX_NOTICES {
            notices.push(notice_at(
                NoticeLevel::Attention,
                &project,
                &index.to_string(),
            ));
        }

        let recent = notices.recent(&project);
        assert_eq!(recent.len(), MAX_ATTENTION_NOTICES + 1);
        assert_eq!(recent[0].message(), "failure");
        assert_eq!(
            recent.last().unwrap().message(),
            &(MAX_NOTICES - 1).to_string()
        );
    }

    /// Attention is transient: only warnings and errors reach the log.
    #[test]
    fn attention_is_not_mirrored_to_the_log() {
        let scratch = TempDir::new().unwrap();
        let log = scratch.path().join("service.log");
        let project = ProjectKey::new(PathBuf::from("/a/muster.yml"));
        let mut notices = Notices::new(Some(log.clone()));

        notices.push(notice_at(NoticeLevel::Attention, &project, "bell"));
        notices.push(notice_at(NoticeLevel::Warning, &project, "warned"));

        let written = fs::read_to_string(log).unwrap();
        assert_eq!(written.lines().count(), 1);
        assert!(written.contains("warned"));
    }

    #[test]
    fn the_ring_keeps_only_the_newest_notices() {
        let project = ProjectKey::new(PathBuf::from("/a/muster.yml"));
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
        let first = ProjectKey::new(PathBuf::from("/a/muster.yml"));
        let second = ProjectKey::new(PathBuf::from("/b/muster.yml"));
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
        let first = ProjectKey::new(PathBuf::from("/a/muster.yml"));
        let second = ProjectKey::new(PathBuf::from("/b/muster.yml"));
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

    #[test]
    fn each_notice_is_mirrored_as_a_json_line() {
        let scratch = TempDir::new().unwrap();
        let log = scratch.path().join("service.log");
        let project = ProjectKey::new(PathBuf::from("/a/muster.yml"));
        let mut notices = Notices::new(Some(log.clone()));

        notices.push(notice(&project, "one"));
        notices.push(notice(&project, "two"));

        let written = fs::read_to_string(log).unwrap();
        let lines: Vec<serde_json::Value> = written
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1]["message"], "two");
        assert_eq!(lines[1]["level"], "error");
        assert_eq!(lines[1]["project"], "/a/muster.yml");
    }
}

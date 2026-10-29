use std::collections::HashMap;

use super::{OccupancyChange, ReportStamp, ShellId, ShellRecord};
use crate::domain::value::ProjectRoot;

/// What the table knows of one shell: its newest report and the project that
/// report put it in.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    /// The newest report applied.
    stamp: ReportStamp,
    /// The project it is in, or `None` outside every project.
    project: Option<ProjectRoot>,
}

impl Seen {
    /// What `record` says of its shell.
    fn of(record: &ShellRecord) -> Self {
        Self {
            stamp: *record.stamp(),
            project: record.project().clone(),
        }
    }
}

/// Which live shell is in which project. A project is occupied while at least
/// one shell is in it; each report says where a shell now is, and the table
/// answers with the projects that gained their first shell or lost their
/// last.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Occupancy {
    shells: HashMap<ShellId, Seen>,
}

impl Occupancy {
    /// A table with no shell in it.
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies `record`, a report that its shell is now in its project; a
    /// report not newer than the shell's last is ignored. Returns a leave
    /// before an enter.
    pub fn report(&mut self, record: &ShellRecord) -> Vec<OccupancyChange> {
        if !self.is_newer(record) {
            return Vec::new();
        }
        let project = record.project().clone();
        let before = self
            .shells
            .insert(*record.shell(), Seen::of(record))
            .and_then(|seen| seen.project);
        if before == project {
            return Vec::new();
        }
        let left = before
            .filter(|root| self.occupants(root) == 0)
            .map(OccupancyChange::Left);
        let entered = project
            .filter(|root| self.occupants(root) == 1)
            .map(OccupancyChange::Entered);
        left.into_iter().chain(entered).collect()
    }

    /// Takes a record as known state: never yields a change. A record not
    /// newer than what the table knows of its shell is ignored.
    pub fn restore(&mut self, record: &ShellRecord) {
        if self.is_newer(record) {
            self.shells.insert(*record.shell(), Seen::of(record));
        }
    }

    /// Forgets `shell`, returning the leave of its project when it was that
    /// project's last shell.
    pub fn remove(&mut self, shell: &ShellId) -> Vec<OccupancyChange> {
        self.shells
            .remove(shell)
            .and_then(|seen| seen.project)
            .filter(|root| self.occupants(root) == 0)
            .map(OccupancyChange::Left)
            .into_iter()
            .collect()
    }

    /// How many shells are in `project`.
    pub fn occupants(&self, project: &ProjectRoot) -> usize {
        self.shells
            .values()
            .filter(|seen| seen.project.as_ref() == Some(project))
            .count()
    }

    /// The project `shell` is in, or `None` when it is outside every project
    /// or unknown.
    pub fn project_of(&self, shell: &ShellId) -> Option<&ProjectRoot> {
        self.shells.get(shell)?.project.as_ref()
    }

    /// Every shell the table knows, in order.
    pub fn shells(&self) -> Vec<ShellId> {
        let mut shells: Vec<ShellId> = self.shells.keys().copied().collect();
        shells.sort_unstable();
        shells
    }

    /// Whether the table knows `shell`.
    pub fn knows(&self, shell: &ShellId) -> bool {
        self.shells.contains_key(shell)
    }

    /// How many shells the table knows, in a project or not.
    pub fn len(&self) -> usize {
        self.shells.len()
    }

    /// Whether the table knows no shell.
    pub fn is_empty(&self) -> bool {
        self.shells.is_empty()
    }

    /// Whether `record` is newer than the last report the table took from
    /// its shell, or the shell is unknown.
    fn is_newer(&self, record: &ShellRecord) -> bool {
        self.shells
            .get(record.shell())
            .is_none_or(|seen| record.stamp().supersedes(&seen.stamp))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::domain::{
        occupancy::testing::stamp,
        value::{Pid, StartToken},
    };

    /// The hook load most tests report from.
    const LOAD: u64 = 1;

    /// The shell running as `pid`, started at `start`.
    fn shell(pid: u32, start: u64) -> ShellId {
        ShellId::builder()
            .pid(Pid::try_new(pid).unwrap())
            .start(StartToken::try_new(start).unwrap())
            .build()
    }

    /// The project folder `/home/me/<name>`.
    fn root(name: &str) -> ProjectRoot {
        ProjectRoot::try_new(PathBuf::from("/home/me").join(name)).unwrap()
    }

    /// `shell`'s report at prompt `seq` of hook load `load` that it is in
    /// the project `project` names.
    fn loaded(shell: ShellId, load: u64, seq: u64, project: Option<&str>) -> ShellRecord {
        ShellRecord::builder()
            .shell(shell)
            .stamp(stamp(load, seq))
            .project(project.map(root))
            .build()
    }

    /// `shell`'s report at prompt `seq` of [`LOAD`].
    fn at(shell: ShellId, seq: u64, project: Option<&str>) -> ShellRecord {
        loaded(shell, LOAD, seq, project)
    }

    #[test]
    fn the_first_shell_in_a_project_enters_it_and_the_last_out_leaves_it() {
        let mut table = Occupancy::new();

        assert_eq!(table.report(&at(shell(100, 1), 1, Some("shop"))), [
            OccupancyChange::Entered(root("shop"))
        ]);
        assert_eq!(table.occupants(&root("shop")), 1);
        assert_eq!(table.report(&at(shell(100, 1), 2, None)), [
            OccupancyChange::Left(root("shop"))
        ]);
        assert_eq!(table.occupants(&root("shop")), 0);
        assert!(table.knows(&shell(100, 1)), "a shell outside every project");
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn a_move_between_projects_is_left_then_entered() {
        let mut table = Occupancy::new();
        table.report(&at(shell(100, 1), 1, Some("shop")));

        let changes = table.report(&at(shell(100, 1), 2, Some("blog")));

        assert_eq!(changes, [
            OccupancyChange::Left(root("shop")),
            OccupancyChange::Entered(root("blog"))
        ]);
        assert!(
            table.report(&at(shell(100, 1), 3, Some("blog"))).is_empty(),
            "a prompt in the same project changes nothing"
        );
    }

    /// Review Focus 4: a report arriving after a newer one is dropped, so
    /// the newer folder wins.
    #[test]
    fn a_report_not_newer_than_the_last_changes_nothing() {
        let mut table = Occupancy::new();
        table.report(&at(shell(100, 1), 5, Some("blog")));

        assert!(table.report(&at(shell(100, 1), 4, Some("shop"))).is_empty());
        assert!(
            table.report(&at(shell(100, 1), 5, None)).is_empty(),
            "the same seq again"
        );

        assert_eq!(table.occupants(&root("blog")), 1);
        assert_eq!(table.occupants(&root("shop")), 0);
    }

    /// A shell that `exec`s itself loads the hook again and counts its
    /// prompts from one: its first report under the new load moves it, and
    /// later prompts of that load follow the usual order.
    #[test]
    fn a_report_of_a_new_hook_load_is_newer_whatever_its_seq() {
        let mut table = Occupancy::new();
        table.report(&loaded(shell(100, 1), 7, 50, Some("blog")));

        assert_eq!(table.report(&loaded(shell(100, 1), 8, 1, Some("shop"))), [
            OccupancyChange::Left(root("blog")),
            OccupancyChange::Entered(root("shop"))
        ]);
        assert!(
            table.report(&loaded(shell(100, 1), 8, 1, None)).is_empty(),
            "the same prompt of the new load again"
        );
        assert_eq!(table.report(&loaded(shell(100, 1), 8, 2, None)), [
            OccupancyChange::Left(root("shop"))
        ]);
    }

    #[test]
    fn a_second_shell_in_a_project_changes_nothing_until_both_go() {
        let mut table = Occupancy::new();
        table.report(&at(shell(100, 1), 1, Some("shop")));

        assert!(table.report(&at(shell(200, 1), 1, Some("shop"))).is_empty());
        assert_eq!(table.occupants(&root("shop")), 2);
        assert!(table.report(&at(shell(100, 1), 2, None)).is_empty());
        assert_eq!(table.remove(&shell(200, 1)), [OccupancyChange::Left(root(
            "shop"
        ))]);
    }

    #[test]
    fn restore_never_yields_a_change() {
        let mut table = Occupancy::new();

        table.restore(&at(shell(100, 1), 7, Some("shop")));

        assert_eq!(table.occupants(&root("shop")), 1);
        assert!(
            table.report(&at(shell(100, 1), 7, Some("blog"))).is_empty(),
            "a report not newer than the record"
        );
        assert!(
            table.report(&at(shell(200, 1), 1, Some("shop"))).is_empty(),
            "the restored shell already occupies shop"
        );
        table.restore(&at(shell(100, 1), 6, None));
        assert_eq!(
            table.occupants(&root("shop")),
            2,
            "an older record is ignored"
        );
    }

    #[test]
    fn removing_the_last_shell_leaves_its_project() {
        let mut table = Occupancy::new();
        table.report(&at(shell(100, 1), 1, Some("shop")));
        table.report(&at(shell(200, 1), 1, None));

        assert_eq!(table.remove(&shell(100, 1)), [OccupancyChange::Left(root(
            "shop"
        ))]);
        assert!(table.remove(&shell(200, 1)).is_empty());
        assert!(table.remove(&shell(300, 1)).is_empty(), "an unknown shell");
        assert!(table.is_empty());
    }

    /// A shell's project follows its newest report and its restored record,
    /// and is gone once it leaves every project or is removed.
    #[test]
    fn the_project_of_a_shell_follows_its_newest_report() {
        let mut table = Occupancy::new();
        assert_eq!(table.project_of(&shell(100, 1)), None, "an unknown shell");

        table.report(&at(shell(100, 1), 1, Some("shop")));
        assert_eq!(table.project_of(&shell(100, 1)), Some(&root("shop")));
        table.report(&at(shell(100, 1), 2, Some("blog")));
        table.report(&at(shell(100, 1), 1, Some("shop")));
        assert_eq!(table.project_of(&shell(100, 1)), Some(&root("blog")));
        table.report(&at(shell(100, 1), 3, None));
        assert_eq!(table.project_of(&shell(100, 1)), None);

        table.restore(&at(shell(200, 1), 4, Some("shop")));
        assert_eq!(table.project_of(&shell(200, 1)), Some(&root("shop")));
        table.remove(&shell(200, 1));
        assert_eq!(table.project_of(&shell(200, 1)), None);
    }

    #[test]
    fn the_same_pid_with_another_start_is_another_shell() {
        let mut table = Occupancy::new();
        table.report(&at(shell(100, 1), 9, Some("shop")));

        assert!(table.report(&at(shell(100, 2), 1, Some("shop"))).is_empty());

        assert_eq!(table.occupants(&root("shop")), 2);
        assert_eq!(table.shells(), [shell(100, 1), shell(100, 2)]);
        assert!(!table.knows(&shell(100, 3)));
    }
}

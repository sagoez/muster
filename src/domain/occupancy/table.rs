use std::collections::{BTreeSet, HashMap};

use super::{OccupancyChange, ShellId, ShellRecord, seen::Seen};
use crate::domain::value::ProjectRoot;

/// Which live shell is in which project. A project is occupied while at least
/// one shell is in it; each report says where a shell now is, and the table
/// answers with the projects that gained their first shell or lost their
/// last. Those changes are the only leaves and entries there are: a count
/// alone never says a shell left.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Occupancy {
    /// What the table knows of each shell.
    shells: HashMap<ShellId, Seen>,
    /// How many shells count in each project that has any.
    counts: HashMap<ProjectRoot, usize>,
}

impl Occupancy {
    /// A table with no shell in it.
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies `record`, a report that its shell is now in its project; a
    /// report not newer than the shell's last is ignored. Returns each leave
    /// before each entry.
    pub fn report(&mut self, record: &ShellRecord) -> Vec<OccupancyChange> {
        if !self.is_newer(record) {
            return Vec::new();
        }
        self.put(record)
    }

    /// Takes a record as known state, as a service starting does: its
    /// shell counts where the record says, and nothing enters or leaves. A
    /// record not newer than what the table knows of its shell is ignored.
    pub fn restore(&mut self, record: &ShellRecord) {
        if self.is_newer(record) {
            self.put(record);
        }
    }

    /// Forgets `shell`, returning the leave of each project it was the last
    /// shell of.
    pub fn remove(&mut self, shell: &ShellId) -> Vec<OccupancyChange> {
        let before = self
            .shells
            .remove(shell)
            .map(|seen| seen.projects().clone())
            .unwrap_or_default();
        self.recount(&before, &BTreeSet::new())
    }

    /// How many shells are in `project`.
    pub fn occupants(&self, project: &ProjectRoot) -> usize {
        self.counts.get(project).copied().unwrap_or_default()
    }

    /// Every shell that counts in `project`, in order.
    pub fn shells_in(&self, project: &ProjectRoot) -> Vec<ShellId> {
        let mut shells: Vec<ShellId> = self
            .shells
            .iter()
            .filter(|(_, seen)| seen.projects().contains(project))
            .map(|(shell, _)| *shell)
            .collect();
        shells.sort_unstable();
        shells
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
    /// its shell, or the shell is unknown, so a report of it is applied.
    pub fn is_newer(&self, record: &ShellRecord) -> bool {
        self.shells
            .get(record.shell())
            .is_none_or(|seen| record.stamp().supersedes(seen.stamp()))
    }

    /// Keeps `record` as what the table knows of its shell, returning the
    /// projects that move made lose their last shell or gain their first.
    fn put(&mut self, record: &ShellRecord) -> Vec<OccupancyChange> {
        let seen = Seen::of(record);
        let after = seen.projects().clone();
        let before = self
            .shells
            .insert(*record.shell(), seen)
            .map(|seen| seen.projects().clone())
            .unwrap_or_default();
        self.recount(&before, &after)
    }

    /// Moves one shell from counting in `before` to counting in `after`.
    /// Returns each project left with no shell, then each that gained its
    /// first.
    fn recount(
        &mut self,
        before: &BTreeSet<ProjectRoot>,
        after: &BTreeSet<ProjectRoot>,
    ) -> Vec<OccupancyChange> {
        let mut changes = Vec::new();
        for root in before.difference(after) {
            let Some(count) = self.counts.get_mut(root) else {
                continue;
            };
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.counts.remove(root);
                changes.push(OccupancyChange::Left(root.clone()));
            }
        }
        for root in after.difference(before) {
            let count = self.counts.entry(root.clone()).or_default();
            *count += 1;
            if *count == 1 {
                changes.push(OccupancyChange::Entered(root.clone()));
            }
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        super::testing::{stamp, test_root, test_shell},
        *,
    };
    use crate::domain::value::EpochNanos;

    /// The hook load most tests report from.
    const LOAD: u64 = 1;
    /// The folder every report is made from; the table never reads it.
    const FOLDER: &str = "/home/me";

    /// `shell`'s report at prompt `seq` of hook load `load` that it is in
    /// the project `project` names.
    fn loaded(shell: ShellId, load: u64, seq: u64, project: Option<&str>) -> ShellRecord {
        ShellRecord::builder()
            .shell(shell)
            .stamp(stamp(load, seq))
            .folder(PathBuf::from(FOLDER))
            .arrived(EpochNanos::new(seq))
            .project(project.map(test_root))
            .build()
    }

    /// `shell`'s report at prompt `seq` of [`LOAD`].
    fn at(shell: ShellId, seq: u64, project: Option<&str>) -> ShellRecord {
        loaded(shell, LOAD, seq, project)
    }

    #[test]
    fn the_first_shell_in_a_project_enters_it_and_the_last_out_leaves_it() {
        let mut table = Occupancy::new();

        assert_eq!(table.report(&at(test_shell(100, 1), 1, Some("shop"))), [
            OccupancyChange::Entered(test_root("shop"))
        ]);
        assert_eq!(table.occupants(&test_root("shop")), 1);
        assert_eq!(table.report(&at(test_shell(100, 1), 2, None)), [
            OccupancyChange::Left(test_root("shop"))
        ]);
        assert_eq!(table.occupants(&test_root("shop")), 0);
        assert!(
            table.knows(&test_shell(100, 1)),
            "a shell outside every project"
        );
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn a_move_between_projects_is_left_then_entered() {
        let mut table = Occupancy::new();
        table.report(&at(test_shell(100, 1), 1, Some("shop")));

        let changes = table.report(&at(test_shell(100, 1), 2, Some("blog")));

        assert_eq!(changes, [
            OccupancyChange::Left(test_root("shop")),
            OccupancyChange::Entered(test_root("blog"))
        ]);
        assert!(
            table
                .report(&at(test_shell(100, 1), 3, Some("blog")))
                .is_empty(),
            "a prompt in the same project changes nothing"
        );
    }

    /// Review Focus 4: a report arriving after a newer one is dropped, so
    /// the newer folder wins.
    #[test]
    fn a_report_not_newer_than_the_last_changes_nothing() {
        let mut table = Occupancy::new();
        table.report(&at(test_shell(100, 1), 5, Some("blog")));

        assert!(
            table
                .report(&at(test_shell(100, 1), 4, Some("shop")))
                .is_empty()
        );
        assert!(
            table.report(&at(test_shell(100, 1), 5, None)).is_empty(),
            "the same seq again"
        );

        assert_eq!(table.occupants(&test_root("blog")), 1);
        assert_eq!(table.occupants(&test_root("shop")), 0);
    }

    /// A shell that `exec`s itself loads the hook again and counts its
    /// prompts from one: its first report under the new load moves it, and
    /// later prompts of that load follow the usual order.
    #[test]
    fn a_report_of_a_new_hook_load_is_newer_whatever_its_seq() {
        let mut table = Occupancy::new();
        table.report(&loaded(test_shell(100, 1), 7, 50, Some("blog")));

        assert_eq!(
            table.report(&loaded(test_shell(100, 1), 8, 1, Some("shop"))),
            [
                OccupancyChange::Left(test_root("blog")),
                OccupancyChange::Entered(test_root("shop"))
            ]
        );
        assert!(
            table
                .report(&loaded(test_shell(100, 1), 8, 1, None))
                .is_empty(),
            "the same prompt of the new load again"
        );
        assert_eq!(table.report(&loaded(test_shell(100, 1), 8, 2, None)), [
            OccupancyChange::Left(test_root("shop"))
        ]);
    }

    /// CR-1: a report of the older load that arrives after the newer load's
    /// is dropped, so a late prompt from before an `exec` never moves the
    /// shell back.
    #[test]
    fn a_late_report_of_an_older_hook_load_changes_nothing() {
        let mut table = Occupancy::new();
        table.report(&loaded(test_shell(100, 1), 8, 1, Some("shop")));

        assert!(
            table
                .report(&loaded(test_shell(100, 1), 7, 50, Some("blog")))
                .is_empty()
        );
        table.restore(&loaded(test_shell(100, 1), 7, 51, None));

        assert_eq!(table.occupants(&test_root("shop")), 1);
        assert_eq!(table.occupants(&test_root("blog")), 0);
    }

    /// The shells counting in a project are those in it and those holding
    /// it, in order.
    #[test]
    fn shells_in_names_every_shell_counting_in_a_project() {
        let mut table = Occupancy::new();
        table.report(&at(test_shell(200, 1), 1, Some("shop")));
        table.report(&at(test_shell(100, 1), 1, Some("shop")));
        table.report(&at(test_shell(300, 1), 1, Some("blog")));

        assert_eq!(table.shells_in(&test_root("shop")), [
            test_shell(100, 1),
            test_shell(200, 1)
        ]);
        assert!(table.shells_in(&test_root("elsewhere")).is_empty());
    }

    #[test]
    fn a_second_shell_in_a_project_changes_nothing_until_both_go() {
        let mut table = Occupancy::new();
        table.report(&at(test_shell(100, 1), 1, Some("shop")));

        assert!(
            table
                .report(&at(test_shell(200, 1), 1, Some("shop")))
                .is_empty()
        );
        assert_eq!(table.occupants(&test_root("shop")), 2);
        assert!(table.report(&at(test_shell(100, 1), 2, None)).is_empty());
        assert_eq!(table.remove(&test_shell(200, 1)), [OccupancyChange::Left(
            test_root("shop")
        )]);
    }

    #[test]
    fn restore_never_yields_a_change() {
        let mut table = Occupancy::new();

        table.restore(&at(test_shell(100, 1), 7, Some("shop")));

        assert_eq!(table.occupants(&test_root("shop")), 1);
        assert!(
            table
                .report(&at(test_shell(100, 1), 7, Some("blog")))
                .is_empty(),
            "a report not newer than the record"
        );
        assert!(
            table
                .report(&at(test_shell(200, 1), 1, Some("shop")))
                .is_empty(),
            "the restored shell already occupies shop"
        );
        table.restore(&at(test_shell(100, 1), 6, None));
        assert_eq!(
            table.occupants(&test_root("shop")),
            2,
            "an older record is ignored"
        );
    }

    #[test]
    fn removing_the_last_shell_leaves_its_project() {
        let mut table = Occupancy::new();
        table.report(&at(test_shell(100, 1), 1, Some("shop")));
        table.report(&at(test_shell(200, 1), 1, None));

        assert_eq!(table.remove(&test_shell(100, 1)), [OccupancyChange::Left(
            test_root("shop")
        )]);
        assert!(table.remove(&test_shell(200, 1)).is_empty());
        assert!(
            table.remove(&test_shell(300, 1)).is_empty(),
            "an unknown shell"
        );
        assert!(table.is_empty());
    }

    /// A shell counts where its newest report or restored record puts it,
    /// and nowhere once it leaves every project or is removed.
    #[test]
    fn a_shell_counts_where_its_newest_report_puts_it() {
        let mut table = Occupancy::new();

        table.report(&at(test_shell(100, 1), 1, Some("shop")));
        table.report(&at(test_shell(100, 1), 2, Some("blog")));
        table.report(&at(test_shell(100, 1), 1, Some("shop")));
        assert_eq!(table.occupants(&test_root("shop")), 0);
        assert_eq!(table.occupants(&test_root("blog")), 1);
        table.report(&at(test_shell(100, 1), 3, None));
        assert_eq!(table.occupants(&test_root("blog")), 0);

        table.restore(&at(test_shell(200, 1), 4, Some("shop")));
        assert_eq!(table.occupants(&test_root("shop")), 1);
        table.remove(&test_shell(200, 1));
        assert_eq!(table.occupants(&test_root("shop")), 0);
    }

    /// A shell holding the outer project a nested one was registered in
    /// counts in both, so the outer one sees no leave until the shell
    /// leaves its tree, and then both see theirs.
    #[test]
    fn a_held_project_counts_the_shell_until_it_lets_go() {
        let mut table = Occupancy::new();
        table.report(&at(test_shell(100, 1), 1, Some("shop")));

        let nested = ShellRecord::builder()
            .shell(test_shell(100, 1))
            .stamp(stamp(LOAD, 2))
            .folder(PathBuf::from(FOLDER))
            .arrived(EpochNanos::new(2))
            .project(Some(test_root("shop/admin")))
            .held(vec![test_root("shop")])
            .build();
        assert_eq!(table.report(&nested), [OccupancyChange::Entered(
            test_root("shop/admin")
        )]);
        assert_eq!(table.occupants(&test_root("shop")), 1, "shop is held");

        assert_eq!(table.report(&at(test_shell(100, 1), 3, None)), [
            OccupancyChange::Left(test_root("shop")),
            OccupancyChange::Left(test_root("shop/admin"))
        ]);
    }

    /// A shell restored from its record and then found gone leaves the
    /// project it was the last in: a service starting sees that leave.
    #[test]
    fn a_restored_shell_removed_leaves_its_project_unless_another_stays() {
        let mut table = Occupancy::new();
        table.restore(&at(test_shell(100, 1), 1, Some("shop")));
        table.restore(&at(test_shell(200, 1), 1, Some("blog")));
        table.restore(&at(test_shell(300, 1), 1, Some("blog")));

        assert_eq!(table.remove(&test_shell(100, 1)), [OccupancyChange::Left(
            test_root("shop")
        )]);
        assert!(
            table.remove(&test_shell(200, 1)).is_empty(),
            "blog still has one"
        );
        assert_eq!(table.occupants(&test_root("blog")), 1);
    }

    #[test]
    fn the_same_pid_with_another_start_is_another_shell() {
        let mut table = Occupancy::new();
        table.report(&at(test_shell(100, 1), 9, Some("shop")));

        assert!(
            table
                .report(&at(test_shell(100, 2), 1, Some("shop")))
                .is_empty()
        );

        assert_eq!(table.occupants(&test_root("shop")), 2);
        assert_eq!(table.shells(), [test_shell(100, 1), test_shell(100, 2)]);
        assert!(!table.knows(&test_shell(100, 3)));
    }
}

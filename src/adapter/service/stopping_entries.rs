use std::collections::HashMap;

use getset::Getters;
use typed_builder::TypedBuilder;

use super::launch_environment::LaunchEnvironment;
use crate::domain::{
    occupancy::ShellId,
    value::{ProcessName, ProjectRoot},
};

/// An entering that found autostart processes still stopping.
#[derive(Debug, Clone, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct StoppingEntry {
    /// The shell that entered.
    shell: ShellId,
    /// The environment it entered with.
    environment: LaunchEnvironment,
    /// The processes still stopping when it entered.
    names: Vec<ProcessName>,
}

/// Enterings that found a project's autostart processes still stopping, as
/// an idle stop that is still running leaves them; each is entered again
/// once those processes have stopped, so a shell back inside finds them up.
#[derive(Debug, Default)]
pub struct StoppingEntries {
    /// The latest such entering of each project.
    waiting: HashMap<ProjectRoot, StoppingEntry>,
}

impl StoppingEntries {
    /// Remembers `entry`, the latest entering of `project` to find its
    /// processes still stopping.
    pub fn wait(&mut self, project: &ProjectRoot, entry: StoppingEntry) {
        self.waiting.insert(project.clone(), entry);
    }

    /// Takes every entering none of whose processes `runs` still says runs,
    /// with its project.
    pub fn take_stopped(
        &mut self,
        runs: impl Fn(&ProjectRoot, &ProcessName) -> bool,
    ) -> Vec<(ProjectRoot, StoppingEntry)> {
        let stopped: Vec<ProjectRoot> = self
            .waiting
            .iter()
            .filter(|(project, entry)| !entry.names.iter().any(|name| runs(project, name)))
            .map(|(project, _)| project.clone())
            .collect();
        stopped
            .into_iter()
            .filter_map(|project| {
                let entry = self.waiting.remove(&project)?;
                Some((project, entry))
            })
            .collect()
    }

    /// Forgets the entering of `project`: its shell left, or a client acted
    /// on it since.
    pub fn forget_project(&mut self, project: &ProjectRoot) {
        self.waiting.remove(project);
    }

    /// Forgets every entering `shell` made, since it is gone.
    pub fn forget_shell(&mut self, shell: &ShellId) {
        self.waiting.retain(|_, entry| entry.shell != *shell);
    }

    /// Whether no entering waits.
    pub fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::runtime_testing::{test_root, test_shell},
        *,
    };

    /// The process named `raw`.
    fn name(raw: &str) -> ProcessName {
        ProcessName::try_new(raw).unwrap()
    }

    /// `shell`'s entering, waiting on `names`.
    fn entry(shell: ShellId, names: &[&str]) -> StoppingEntry {
        StoppingEntry::builder()
            .shell(shell)
            .environment(LaunchEnvironment::default())
            .names(names.iter().map(|raw| name(raw)).collect())
            .build()
    }

    #[test]
    fn an_entering_is_taken_once_every_process_it_waits_on_has_stopped() {
        let mut entries = StoppingEntries::default();
        entries.wait(
            &test_root("shop"),
            entry(test_shell(101, 1), &["api", "web"]),
        );

        assert!(
            entries
                .take_stopped(|_, name| *name == self::name("web"))
                .is_empty()
        );
        assert_eq!(entries.take_stopped(|_, _| false), [(
            test_root("shop"),
            entry(test_shell(101, 1), &["api", "web"])
        )]);
        assert!(entries.is_empty(), "taken once");
    }

    #[test]
    fn a_leave_an_action_or_a_gone_shell_forgets_its_entering() {
        let mut entries = StoppingEntries::default();
        entries.wait(&test_root("shop"), entry(test_shell(101, 1), &["api"]));
        entries.wait(&test_root("blog"), entry(test_shell(102, 1), &["web"]));

        entries.forget_project(&test_root("shop"));
        entries.forget_shell(&test_shell(102, 1));

        assert!(entries.is_empty());
    }
}

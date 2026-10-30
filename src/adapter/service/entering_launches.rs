use std::collections::HashMap;

use crate::domain::{
    occupancy::ShellId,
    value::{ProcessName, ProjectRoot},
};

/// Which shell's entering started each launch still in flight, so a port
/// block its worker finds after the answer reaches that shell.
#[derive(Debug, Default)]
pub struct EnteringLaunches {
    /// The entering shell of each launching process, by project and name.
    launches: HashMap<(ProjectRoot, ProcessName), ShellId>,
}

impl EnteringLaunches {
    /// Records that `shell` entering `project` launched `names`.
    pub fn begin(&mut self, project: &ProjectRoot, names: &[ProcessName], shell: ShellId) {
        for name in names {
            self.launches.insert((project.clone(), name.clone()), shell);
        }
    }

    /// Ends the launch of `name` in `project`, returning the shell whose
    /// entering started it, if one did.
    pub fn settle(&mut self, project: &ProjectRoot, name: &ProcessName) -> Option<ShellId> {
        self.launches.remove(&(project.clone(), name.clone()))
    }

    /// Forgets every launch `project`'s processes have in flight: a client
    /// acted on the project since, so a later block is its own.
    pub fn forget_project(&mut self, project: &ProjectRoot) {
        self.launches.retain(|(root, _), _| root != project);
    }

    /// Forgets every launch `shell` started, since it is gone.
    pub fn forget_shell(&mut self, shell: &ShellId) {
        self.launches.retain(|_, entering| entering != shell);
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

    #[test]
    fn a_launch_settles_once_to_the_shell_that_entered() {
        let mut launches = EnteringLaunches::default();
        launches.begin(
            &test_root("shop"),
            &[name("api"), name("web")],
            test_shell(101, 1),
        );

        assert_eq!(
            launches.settle(&test_root("shop"), &name("api")),
            Some(test_shell(101, 1))
        );
        assert_eq!(launches.settle(&test_root("shop"), &name("api")), None);
        assert_eq!(launches.settle(&test_root("blog"), &name("web")), None);

        launches.forget_shell(&test_shell(101, 1));
        assert_eq!(launches.settle(&test_root("shop"), &name("web")), None);
    }

    #[test]
    fn a_client_acting_on_the_project_takes_its_launches_over() {
        let mut launches = EnteringLaunches::default();
        launches.begin(&test_root("shop"), &[name("api")], test_shell(101, 1));
        launches.begin(&test_root("blog"), &[name("api")], test_shell(101, 1));

        launches.forget_project(&test_root("shop"));

        assert_eq!(launches.settle(&test_root("shop"), &name("api")), None);
        assert_eq!(
            launches.settle(&test_root("blog"), &name("api")),
            Some(test_shell(101, 1))
        );
    }
}

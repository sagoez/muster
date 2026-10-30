use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::domain::{project::Project, value::EpochNanos};

/// A registered project with when it was registered.
#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct RegisteredProject {
    /// The project.
    #[getset(get = "pub")]
    project: Project,
    /// When it was registered, or `None` when its stored time does not read.
    #[getset(get_copy = "pub")]
    registered: Option<EpochNanos>,
}

impl RegisteredProject {
    /// Whether it was registered after a shell arrived in its folder at
    /// `arrived`, so around that shell; a time that does not read exempts
    /// no shell.
    pub fn registered_after(&self, arrived: EpochNanos) -> bool {
        self.registered.is_some_and(|at| at > arrived)
    }
}

#[cfg(test)]
mod tests {
    use super::{super::testing::test_root, *};
    use crate::domain::value::ProjectName;

    /// The project `shop`, registered at `at`.
    fn shop(at: Option<u64>) -> RegisteredProject {
        RegisteredProject::builder()
            .project(
                Project::builder()
                    .root(test_root("shop"))
                    .name(ProjectName::try_new("shop").unwrap())
                    .build(),
            )
            .registered(at.map(EpochNanos::new))
            .build()
    }

    /// Only a project registered after the shell arrived was registered
    /// around it, and one whose time does not read was registered around
    /// no shell.
    #[test]
    fn a_project_is_registered_around_shells_that_arrived_before_it() {
        let arrived = EpochNanos::new(50);

        assert!(shop(Some(60)).registered_after(arrived));
        assert!(!shop(Some(50)).registered_after(arrived));
        assert!(!shop(Some(40)).registered_after(arrived));
        assert!(!shop(None).registered_after(arrived));
    }
}

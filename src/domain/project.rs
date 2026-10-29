use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::value::{IdleStop, ProjectName, ProjectRoot};

/// A registered project: a folder, which is its identity, and its display name.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Project {
    /// The canonical folder the project is.
    root: ProjectRoot,
    /// The name the CLI shows and accepts for it.
    name: ProjectName,
    /// Its own idle stop, or `None` to follow the setting.
    #[builder(default)]
    idle_stop: Option<IdleStop>,
}

impl Project {
    /// The idle stop that applies to it: its own, else `setting`.
    pub fn idle_stop_or(&self, setting: IdleStop) -> IdleStop {
        self.idle_stop.unwrap_or(setting)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// The project `shop`, with `idle_stop` as its own.
    fn shop(idle_stop: Option<IdleStop>) -> Project {
        Project::builder()
            .root(ProjectRoot::try_new(PathBuf::from("/home/me/shop")).unwrap())
            .name(ProjectName::try_new("shop").unwrap())
            .idle_stop(idle_stop)
            .build()
    }

    #[test]
    fn a_project_override_wins_over_the_setting() {
        let setting: IdleStop = "30m".parse().unwrap();
        let own: IdleStop = "5m".parse().unwrap();

        assert_eq!(shop(None).idle_stop_or(setting), setting);
        assert_eq!(shop(Some(own)).idle_stop_or(setting), own);
        assert_eq!(
            shop(Some(IdleStop::Never)).idle_stop_or(setting),
            IdleStop::Never
        );
    }
}

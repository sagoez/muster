use std::path::PathBuf;

use getset::Getters;
use typed_builder::TypedBuilder;

use super::{ReportStamp, ShellId};
use crate::domain::value::ProjectRoot;

/// A shell's report, as the service is told it and as it is kept between
/// services: which shell, which hook load and prompt, its folder, and the
/// projects it counts in.
#[derive(Debug, Clone, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ShellRecord {
    /// The shell that reported.
    shell: ShellId,
    /// The hook load and prompt the report was made at.
    stamp: ReportStamp,
    /// The folder the shell was in.
    folder: PathBuf,
    /// The project its folder was in, or `None` outside every project.
    #[builder(default)]
    project: Option<ProjectRoot>,
    /// Projects the shell still counts in though its project is one nested
    /// inside them, since that one was registered around the shell rather
    /// than entered; empty for a shell that moved where it is.
    #[builder(default)]
    held: Vec<ProjectRoot>,
}

impl ShellRecord {
    /// This report as it follows `previous`, the shell's last stored one:
    /// a project registered around the shell nests it in a new project
    /// without it leaving the one it was in, which it holds, as it holds
    /// what `previous` held, for as long as its project stays nested inside.
    pub fn following(self, previous: Option<&Self>) -> Self {
        let Some(previous) = previous else {
            return self;
        };
        let mut held = previous.held.clone();
        if self.registered_since(previous) {
            held.extend(previous.project.clone());
        }
        held.retain(|root| {
            self.project
                .as_ref()
                .is_some_and(|project| project != root && root.contains(project.as_ref()))
        });
        held.sort();
        held.dedup();
        Self { held, ..self }
    }

    /// Whether this report may enter its project and start it: it names a
    /// project `previous`, the shell's last stored report, did not, and that
    /// project was not registered around the shell since.
    pub fn may_enter(&self, previous: Option<&Self>) -> bool {
        self.project.is_some()
            && previous.is_none_or(|previous| {
                previous.project != self.project && !self.registered_since(previous)
            })
    }

    /// Whether this report's project was registered while the shell was
    /// already inside it: `previous` was made from a folder inside it, yet
    /// named no project or only one this project is nested in.
    fn registered_since(&self, previous: &Self) -> bool {
        let Some(project) = &self.project else {
            return false;
        };
        project.contains(&previous.folder)
            && previous
                .project
                .as_ref()
                .is_none_or(|before| before != project && before.contains(project.as_ref()))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::testing::{stamp, test_root, test_shell},
        *,
    };

    /// The report of prompt `seq` from the folder of the test project
    /// `folder` would name, in the project `project` names.
    fn at(seq: u64, folder: &str, project: Option<&str>) -> ShellRecord {
        ShellRecord::builder()
            .shell(test_shell(100, 1))
            .stamp(stamp(1, seq))
            .folder(test_root(folder).as_ref().to_path_buf())
            .project(project.map(test_root))
            .build()
    }

    /// Only a report naming a project the last one did not may enter it;
    /// a prompt in the same project, or one outside every project, may not.
    #[test]
    fn only_a_report_naming_a_new_project_may_enter() {
        let outside = at(1, "elsewhere", None);
        let in_shop = at(1, "shop", Some("shop"));

        assert!(at(1, "shop", Some("shop")).may_enter(None));
        assert!(at(2, "shop", Some("shop")).may_enter(Some(&outside)));
        assert!(at(2, "blog", Some("blog")).may_enter(Some(&in_shop)));
        assert!(!at(2, "shop/src", Some("shop")).may_enter(Some(&in_shop)));
        assert!(!at(2, "elsewhere", None).may_enter(Some(&in_shop)));
        assert!(!outside.may_enter(None));
    }

    /// Correctness M1: a project registered around a shell already in its
    /// folder is no entry, though the shell now counts in it.
    #[test]
    fn a_project_registered_around_the_shell_is_not_entered() {
        let before = at(1, "fresh", None);

        let registered = at(2, "fresh", Some("fresh")).following(Some(&before));

        assert!(!registered.may_enter(Some(&before)));
        assert!(registered.held().is_empty());
        assert!(
            at(2, "fresh/src", Some("fresh")).may_enter(Some(&at(1, "elsewhere", None))),
            "walking in from outside is an entry"
        );
    }

    /// A project registered inside the one the shell is in keeps the outer
    /// one held while the shell stays in its tree, and lets go once it
    /// leaves; moving out to the outer project is an ordinary move.
    #[test]
    fn a_nested_registration_holds_the_outer_project_while_the_shell_stays() {
        let in_outer = at(1, "shop/admin", Some("shop"));

        let nested = at(2, "shop/admin", Some("shop/admin")).following(Some(&in_outer));
        assert!(!nested.may_enter(Some(&in_outer)));
        assert_eq!(nested.held(), &[test_root("shop")]);

        let deeper = at(3, "shop/admin/src", Some("shop/admin")).following(Some(&nested));
        assert_eq!(deeper.held(), &[test_root("shop")], "still in its tree");

        let back = at(4, "shop", Some("shop")).following(Some(&deeper));
        assert!(back.may_enter(Some(&deeper)));
        assert!(back.held().is_empty(), "the shell counts in shop directly");

        let out = at(4, "elsewhere", None).following(Some(&deeper));
        assert!(out.held().is_empty());
    }

    /// A shell in a nested project that moves to its outer project moved
    /// there: that is an entry of the outer project, holding nothing.
    #[test]
    fn moving_from_a_nested_project_to_its_outer_one_is_an_entry() {
        let in_inner = at(1, "shop/admin", Some("shop/admin"));

        let moved = at(2, "shop", Some("shop")).following(Some(&in_inner));

        assert!(moved.may_enter(Some(&in_inner)));
        assert!(moved.held().is_empty());
    }
}

//! The CLI's words for freeing a port a start found held.

use crate::{
    adapter::{
        client::{PortBlockView, PortOwnerView},
        service::{PortBlock, PortOwner},
    },
    domain::value::{ProcessName, ProjectName},
};

/// The command that frees a port another project's process holds, before
/// that project's name.
const DOWN_HINT: &str = "muster down -p";
/// The command that frees a port another process of the same project holds,
/// before that process's name.
const STOP_HINT: &str = "muster stop";
/// What follows the quoted command in a hint.
const FREES_IT: &str = "frees it";

/// How to free the port `block` names, for a start in `own_project`: bring
/// down another project holding it, or stop the process of the same project
/// holding it; none for a process muster does not run. A name is
/// shell-quoted when it needs to be; one no shell word can carry gives no
/// hint.
pub fn block_hint(block: &PortBlock, own_project: &ProjectName) -> Option<String> {
    let PortOwner::Supervised(holder) = block.holder() else {
        return None;
    };
    supervised_hint(holder.project(), holder.process(), own_project)
}

/// [`block_hint`] for the block a report carries as its view.
pub fn block_view_hint(block: &PortBlockView, own_project: &ProjectName) -> Option<String> {
    let PortOwnerView::Supervised { project, process } = block.holder() else {
        return None;
    };
    supervised_hint(project, process, own_project)
}

/// How to free a port that `process` of `project` holds, for a start in
/// `own_project`.
fn supervised_hint(
    project: &ProjectName,
    process: &ProcessName,
    own_project: &ProjectName,
) -> Option<String> {
    let (command, target) = if project == own_project {
        (STOP_HINT, process.as_ref())
    } else {
        (DOWN_HINT, project.as_ref())
    };
    let quoted = shlex::try_quote(target).ok()?;
    Some(format!("`{command} {quoted}` {FREES_IT}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        adapter::service::PortHolder,
        domain::{
            port::PortOccupant,
            value::{LocalPort, Pid},
        },
    };

    /// The block of port 5173 by `owner`.
    fn block(owner: PortOwner) -> PortBlock {
        PortBlock::builder()
            .port(LocalPort::try_new(5173).unwrap())
            .holder(owner)
            .build()
    }

    /// A port held by process `process` of project `project`.
    fn held_by(project: &str, process: &str) -> PortBlock {
        block(PortOwner::Supervised(
            PortHolder::builder()
                .project(ProjectName::try_new(project).unwrap())
                .process(ProcessName::try_new(process).unwrap())
                .build(),
        ))
    }

    /// The project called `name`.
    fn project(name: &str) -> ProjectName {
        ProjectName::try_new(name).unwrap()
    }

    #[test]
    fn the_hint_is_down_for_another_project_stop_for_this_one_and_none_for_others() {
        assert_eq!(
            block_hint(&held_by("admin", "web"), &project("shop")).as_deref(),
            Some("`muster down -p admin` frees it")
        );
        assert_eq!(
            block_hint(&held_by("shop", "web"), &project("shop")).as_deref(),
            Some("`muster stop web` frees it")
        );
        let outside = block(PortOwner::Other(
            PortOccupant::builder()
                .pid(Pid::try_new(4242).unwrap())
                .command("python3 -m http.server 5173".to_string())
                .build(),
        ));
        assert_eq!(block_hint(&outside, &project("shop")), None);
    }

    /// A report's view of a block gives the same hint as the block itself.
    #[test]
    fn the_view_of_a_block_gives_the_same_hint() {
        let outside = block(PortOwner::Other(
            PortOccupant::builder()
                .pid(Pid::try_new(4242).unwrap())
                .command("python3".to_string())
                .build(),
        ));
        for held in [
            held_by("admin site", "web"),
            held_by("shop", "dev server"),
            outside,
        ] {
            assert_eq!(
                block_view_hint(&PortBlockView::from(&held), &project("shop")),
                block_hint(&held, &project("shop")),
                "{held}"
            );
        }
    }

    #[test]
    fn a_project_or_process_name_with_spaces_is_quoted_in_the_hint() {
        assert_eq!(
            block_hint(&held_by("admin site", "web"), &project("shop")).as_deref(),
            Some("`muster down -p 'admin site'` frees it")
        );
        assert_eq!(
            block_hint(&held_by("shop", "dev server"), &project("shop")).as_deref(),
            Some("`muster stop 'dev server'` frees it")
        );
    }
}

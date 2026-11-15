//! The words a pasted command names a project or a process with, so the
//! command line reads each back as that name, even one that reads as a flag
//! or as a folder.

use std::path::Path;

use muster_domain::{
    project::Project,
    value::{ProcessName, ProjectName},
};
use muster_os::path::{home_relative, names_a_folder};

/// The flag naming the project, before its value.
const PROJECT_FLAG: &str = "-p";
/// The flag naming the project, joined to a value that reads as a flag.
const PROJECT_FLAG_JOINED: &str = "--project=";
/// What starts a word the command line would read as a flag.
const FLAG_START: char = '-';
/// Ends the options, so a name that reads as a flag is taken as a name.
const END_OF_OPTIONS: &str = "--";

/// `-p <project>`, or `--project=<project>` when the name reads as a flag,
/// the name shell-quoted when it needs to be; none for a name no shell word
/// can carry. A name that reads as a folder is taken for one wherever the
/// command runs, so where that matters use [`project_option_for`].
#[must_use]
pub fn project_option(project: &ProjectName) -> Option<String> {
    let quoted = shlex::try_quote(project.as_ref()).ok()?;
    Some(if project.as_ref().starts_with(FLAG_START) {
        format!("{PROJECT_FLAG_JOINED}{quoted}")
    } else {
        format!("{PROJECT_FLAG} {quoted}")
    })
}

/// The `-p` words that name `project` from any folder: its name, as
/// [`project_option`] writes it, unless `-p` would take that name for a
/// folder under the one the command runs in, as it does one that starts
/// with `.` or `~` or holds a `/`; then its own folder, written from `~` when
/// it is under `home`, which `-p` expands back. None for a name or a folder
/// no shell word can carry.
#[must_use]
pub fn project_option_for(project: &Project, home: Option<&Path>) -> Option<String> {
    if !names_a_folder(project.name().as_ref()) {
        return project_option(project.name());
    }
    let root = project.root().as_ref();
    root.to_str()?;
    let folder = home_relative(root, home);
    let quoted = shlex::try_quote(&folder).ok()?;
    Some(format!("{PROJECT_FLAG} {quoted}"))
}

/// `<name>`, or `-- <name>` when it reads as a flag, the name shell-quoted
/// when it needs to be; none for a name no shell word can carry.
#[must_use]
pub fn process_operand(name: &ProcessName) -> Option<String> {
    let quoted = shlex::try_quote(name.as_ref()).ok()?;
    Some(if name.as_ref().starts_with(FLAG_START) {
        format!("{END_OF_OPTIONS} {quoted}")
    } else {
        quoted.into_owned()
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use muster_domain::value::ProjectRoot;

    use super::*;

    /// The home folder the tests' projects live under.
    const HOME: &str = "/home/me";

    /// The project named `name` in the folder `root`.
    fn project_at(name: &str, root: &str) -> Project {
        Project::builder()
            .root(ProjectRoot::try_new(PathBuf::from(root)).unwrap())
            .name(ProjectName::try_new(name).unwrap())
            .build()
    }

    /// A plain name stays bare, one a shell would split is quoted, and one
    /// that reads as a flag is passed so it stays a value.
    #[test]
    fn names_read_back_as_themselves() {
        let project = |raw| ProjectName::try_new(raw).unwrap();
        let process = |raw| ProcessName::try_new(raw).unwrap();

        assert_eq!(project_option(&project("shop")).as_deref(), Some("-p shop"));
        assert_eq!(
            project_option(&project("my shop")).as_deref(),
            Some("-p 'my shop'")
        );
        assert_eq!(
            project_option(&project("-x")).as_deref(),
            Some("--project=-x")
        );
        assert_eq!(process_operand(&process("web")).as_deref(), Some("web"));
        assert_eq!(
            process_operand(&process("dev server")).as_deref(),
            Some("'dev server'")
        );
        assert_eq!(
            process_operand(&process("--follow")).as_deref(),
            Some("-- --follow")
        );
    }

    /// A project named like a folder is named by its own folder, from `~`
    /// when it is under the home folder, so `-p` never takes the name for a
    /// folder under wherever the command is pasted; a plain name stays its
    /// name.
    #[test]
    fn a_name_that_reads_as_a_folder_is_named_by_its_folder() {
        let home = Some(Path::new(HOME));
        let named = |name, root| project_option_for(&project_at(name, root), home);

        assert_eq!(
            named("shop", "/home/me/code/shop").as_deref(),
            Some("-p shop")
        );
        assert_eq!(
            named(".devcontainer", "/home/me/code/.devcontainer").as_deref(),
            Some("-p '~/code/.devcontainer'")
        );
        assert_eq!(
            named("~shop", "/home/me/my shop").as_deref(),
            Some("-p '~/my shop'")
        );
        assert_eq!(named("a/b", "/srv/a-b").as_deref(), Some("-p /srv/a-b"));
    }
}

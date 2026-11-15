//! The words a pasted command names a project or a process with, so the
//! command line reads each back as that name, even one that reads as a flag.

use super::{process_name::ProcessName, project_name::ProjectName};

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
/// can carry.
#[must_use]
pub fn project_option(project: &ProjectName) -> Option<String> {
    let quoted = shlex::try_quote(project.as_ref()).ok()?;
    Some(if project.as_ref().starts_with(FLAG_START) {
        format!("{PROJECT_FLAG_JOINED}{quoted}")
    } else {
        format!("{PROJECT_FLAG} {quoted}")
    })
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
    use super::*;

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
}

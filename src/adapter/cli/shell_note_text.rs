//! The lines a shell's hook prints at its next prompt for what the service
//! said about its entering a project.

use super::block_hint::block_hint;
use crate::adapter::{sanitize::printable, service::ShellNote};

/// Opens every line, so a person sees which program spoke.
const NOTE_PREFIX: &str = "muster:";
/// Before the processes entering a project started.
const STARTING: &str = "starting";
/// Between the processes named on a line.
const NAME_SEPARATOR: &str = ", ";
/// Between a refusal and how to free the port it names.
const HINT_SEPARATOR: &str = "; ";

/// One printable line per note: what entering a project started, and each
/// process it would have started that was refused, with how to free a held
/// port. Every line is made printable, so no name or command line a note
/// carries can drive the terminal.
pub fn note_lines(notes: &[ShellNote]) -> Vec<String> {
    notes
        .iter()
        .map(|note| printable(&format!("{NOTE_PREFIX} {}", note_text(note))))
        .collect()
}

/// What `note` says, after the prefix: `shop: starting api, worker`, or
/// `shop: web did not start: ...; `muster down -p admin` frees it`.
fn note_text(note: &ShellNote) -> String {
    match note {
        ShellNote::Started { project, names } => {
            let names: Vec<&str> = names.iter().map(AsRef::as_ref).collect();
            format!("{project}: {STARTING} {}", names.join(NAME_SEPARATOR))
        },
        ShellNote::Refused { project, refusal } => {
            let refused = format!("{project}: {} {}", refusal.name(), refusal.reason());
            match refusal
                .block()
                .as_ref()
                .and_then(|block| block_hint(block, project))
            {
                Some(hint) => format!("{refused}{HINT_SEPARATOR}{hint}"),
                None => refused,
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        adapter::service::{PortBlock, PortHolder, PortOwner, Refusal},
        domain::{
            port::{OCCUPANT_COMMAND_CUT, OCCUPANT_COMMAND_MAX_CHARS, PortOccupant},
            value::{LocalPort, Pid, ProcessName, ProjectName},
        },
    };

    /// The project `name`.
    fn project(name: &str) -> ProjectName {
        ProjectName::try_new(name).unwrap()
    }

    /// The process `name`.
    fn process(name: &str) -> ProcessName {
        ProcessName::try_new(name).unwrap()
    }

    /// `web` of `shop` refused because `owner` holds port 5173.
    fn blocked(owner: PortOwner) -> ShellNote {
        let block = PortBlock::builder()
            .port(LocalPort::try_new(5173).unwrap())
            .holder(owner)
            .build();
        ShellNote::Refused {
            project: project("shop"),
            refusal: Refusal::builder()
                .name(process("web"))
                .reason(block.to_string())
                .block(Some(block))
                .build(),
        }
    }

    /// Process `process` of project `project`, as a port's holder.
    fn supervised(project_name: &str, process_name: &str) -> PortOwner {
        PortOwner::Supervised(
            PortHolder::builder()
                .project(project(project_name))
                .process(process(process_name))
                .build(),
        )
    }

    /// A program muster does not run, as a port's holder.
    fn outsider(command: &str) -> PortOwner {
        PortOwner::Other(
            PortOccupant::builder()
                .pid(Pid::try_new(4242).unwrap())
                .command(command.to_string())
                .build(),
        )
    }

    /// A start line, then a refusal by each kind of holder: another
    /// project's process, one of the same project, and a program muster
    /// does not run, the last with no hint.
    #[test]
    fn note_lines_snapshot() {
        let notes = [
            ShellNote::Started {
                project: project("shop"),
                names: vec![process("api"), process("worker")],
            },
            blocked(supervised("admin", "web")),
            blocked(supervised("shop", "dev server")),
            blocked(outsider("python3 -m http.server 5173")),
            ShellNote::Refused {
                project: project("shop"),
                refusal: Refusal::builder()
                    .name(process("db"))
                    .reason(
                        "cannot start: the process was left running unsupervised as pid 77"
                            .to_string(),
                    )
                    .build(),
            },
        ];

        insta::assert_snapshot!(note_lines(&notes).join("\n"));
    }

    /// A control character a holder's command line or a refusal's reason
    /// carries is escaped, never sent to the terminal, and every note stays
    /// one line.
    #[test]
    fn control_characters_in_a_note_are_escaped() {
        let lines = note_lines(&[
            blocked(outsider("evil\u{1b}]0;owned\u{7}\nrm -rf ~")),
            ShellNote::Refused {
                project: project("shop"),
                refusal: Refusal::builder()
                    .name(process("db"))
                    .reason("gone\u{1b}[2J\r\u{9b}31m".to_string())
                    .build(),
            },
        ]);

        assert_eq!(lines.len(), 2);
        for line in &lines {
            assert!(!line.contains('\u{1b}'), "{line:?}");
            assert!(!line.contains('\u{7}'), "{line:?}");
            assert!(!line.contains('\n'), "{line:?}");
            assert!(!line.contains('\r'), "{line:?}");
            assert!(!line.contains('\u{9b}'), "{line:?}");
        }
        assert!(lines[0].contains("evil\\u{1b}]0;owned"), "{}", lines[0]);
    }

    /// A holder's command line as long as an argv may be is cut to the cap
    /// and marked, so the note stays a short line.
    #[test]
    fn a_huge_holder_command_line_is_cut_in_its_note() {
        let huge = "x".repeat(OCCUPANT_COMMAND_MAX_CHARS * 1000);

        let lines = note_lines(&[blocked(outsider(&huge))]);

        let shown = format!(
            "{}{OCCUPANT_COMMAND_CUT}",
            "x".repeat(OCCUPANT_COMMAND_MAX_CHARS)
        );
        assert!(lines[0].contains(&shown), "{}", lines[0]);
        assert!(
            lines[0].chars().count() < OCCUPANT_COMMAND_MAX_CHARS * 2,
            "{}",
            lines[0]
        );
    }
}

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::domain::value::Pid;

/// Most characters of an occupant's command line kept, so a program with a
/// huge argv never floods a note, `ps` or an MCP answer.
pub const OCCUPANT_COMMAND_MAX_CHARS: usize = 256;
/// Ends a command line cut at [`OCCUPANT_COMMAND_MAX_CHARS`].
pub const OCCUPANT_COMMAND_CUT: &str = "…";

/// A process listening on a TCP port, found in the OS socket table.
#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct PortOccupant {
    /// Its process id.
    #[getset(get_copy = "pub")]
    pid: Pid,
    /// Its command line, or its executable when the command line is unknown,
    /// cut to [`OCCUPANT_COMMAND_MAX_CHARS`].
    #[getset(get = "pub")]
    #[builder(setter(transform = |command: String| capped_command(command)))]
    command: String,
    /// Its parent, that parent's parent and so on, as far as they were found,
    /// so a listener a supervised process spawned is told to be its.
    #[getset(get = "pub")]
    #[builder(default)]
    ancestors: Vec<Pid>,
}

impl PortOccupant {
    /// Its pid, then its ancestors': every process it may have been
    /// started under.
    pub fn lineage(&self) -> Vec<Pid> {
        let mut lineage = vec![self.pid];
        lineage.extend(self.ancestors.iter().copied());
        lineage
    }
}

/// `command` as is when it fits [`OCCUPANT_COMMAND_MAX_CHARS`], else its
/// first that many characters and [`OCCUPANT_COMMAND_CUT`].
fn capped_command(command: String) -> String {
    match command.char_indices().nth(OCCUPANT_COMMAND_MAX_CHARS) {
        Some((cut, _)) => format!("{}{OCCUPANT_COMMAND_CUT}", &command[..cut]),
        None => command,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An occupant of `command`.
    fn occupant(command: String) -> PortOccupant {
        PortOccupant::builder()
            .pid(Pid::try_new(7).unwrap())
            .command(command)
            .build()
    }

    /// A command line up to the cap is kept whole; a longer one, counted in
    /// characters, is cut there and marked.
    #[test]
    fn a_long_command_line_is_cut_and_marked() {
        let fits = "é".repeat(OCCUPANT_COMMAND_MAX_CHARS);
        assert_eq!(occupant(fits.clone()).command(), &fits);

        let cut = occupant(format!("{fits}tail")).command().clone();
        assert_eq!(cut, format!("{fits}{OCCUPANT_COMMAND_CUT}"));
        assert_eq!(
            cut.chars().count(),
            OCCUPANT_COMMAND_MAX_CHARS + OCCUPANT_COMMAND_CUT.chars().count()
        );
    }
}

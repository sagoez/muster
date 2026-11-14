use nutype::nutype;

/// Most characters a process's command holds, well under the kernel's
/// single-argument limit.
pub const COMMAND_MAX_CHARS: usize = 8_192;

/// The shell command used to launch a process: trimmed, non-empty and at most
/// [`COMMAND_MAX_CHARS`] long, so one write cannot flood every listing that
/// shows it.
#[nutype(
    sanitize(trim),
    validate(not_empty, len_char_max = COMMAND_MAX_CHARS),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        AsRef,
        Display,
        Serialize,
        Deserialize
    )
)]
pub struct CommandLine(String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_inner_spacing_but_trims_edges() {
        let cmd = CommandLine::try_new("  npm run dev  ").unwrap();
        assert_eq!(cmd.as_ref(), "npm run dev");
    }

    #[test]
    fn rejects_blank() {
        assert!(CommandLine::try_new("   ").is_err());
    }

    /// A command up to the maximum is kept; one past it is refused.
    #[test]
    fn a_command_is_bounded() {
        assert!(CommandLine::try_new("x".repeat(COMMAND_MAX_CHARS)).is_ok());
        assert_eq!(
            CommandLine::try_new("x".repeat(COMMAND_MAX_CHARS + 1)),
            Err(CommandLineError::LenCharMaxViolated)
        );
    }
}

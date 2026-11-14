use clap::ValueEnum;
use strum::{Display, EnumIter, EnumMessage, EnumProperty};

/// The property naming the file a shell's hook line belongs in.
const DESTINATION: &str = "destination";

/// Shells supported by the dynamic completion engine, named as their
/// binaries are, so `$SHELL` reads as one. A shell's message is its hook
/// line, as documented by clap_complete's CompleteEnv.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Display, EnumIter, EnumMessage, EnumProperty,
)]
#[strum(serialize_all = "kebab-case")]
pub enum CompletionShell {
    #[strum(
        message = "source <(COMPLETE=bash muster)",
        props(destination = "~/.bashrc")
    )]
    Bash,
    #[strum(
        message = "source <(COMPLETE=zsh muster)",
        props(destination = "~/.zshrc")
    )]
    Zsh,
    #[strum(
        message = "COMPLETE=fish muster | source",
        props(destination = "~/.config/fish/completions/muster.fish")
    )]
    Fish,
    #[strum(
        message = "eval (E:COMPLETE=elvish muster | slurp)",
        props(destination = "~/.elvish/rc.elv")
    )]
    Elvish,
    #[value(alias = "pwsh")]
    #[strum(
        message = "$env:COMPLETE = \"powershell\"; muster | Out-String | Invoke-Expression; Remove-Item Env:\\COMPLETE",
        props(destination = "$PROFILE")
    )]
    Powershell,
}

/// The comment naming the destination file, then the hook line itself.
pub fn registration(shell: CompletionShell) -> [String; 2] {
    [
        format!(
            "# add to {}:",
            shell.get_str(DESTINATION).unwrap_or_default()
        ),
        shell.get_message().unwrap_or_default().to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    /// Each shell yields its documented dynamic-completion hook line and
    /// destination.
    #[test]
    fn every_shell_has_its_registration_line_and_destination() {
        for shell in CompletionShell::iter() {
            assert!(shell.get_message().is_some(), "{shell}");
            assert!(shell.get_str(DESTINATION).is_some(), "{shell}");
        }
        assert_eq!(
            CompletionShell::Bash.get_message(),
            Some("source <(COMPLETE=bash muster)")
        );
        assert_eq!(
            CompletionShell::Zsh.get_message(),
            Some("source <(COMPLETE=zsh muster)")
        );
        assert_eq!(
            CompletionShell::Fish.get_message(),
            Some("COMPLETE=fish muster | source")
        );
        assert_eq!(
            CompletionShell::Elvish.get_message(),
            Some("eval (E:COMPLETE=elvish muster | slurp)")
        );
        assert!(
            CompletionShell::Powershell
                .get_message()
                .is_some_and(|line| line.contains("COMPLETE"))
        );
    }

    /// The printed output pairs a destination hint with the line.
    #[test]
    fn output_names_the_destination_file() {
        let [hint, line] = registration(CompletionShell::Zsh);
        assert!(hint.contains(".zshrc"));
        assert_eq!(Some(line.as_str()), CompletionShell::Zsh.get_message());
    }
}

use strum::Display;

/// Shells supported by the dynamic completion engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, Display)]
#[strum(serialize_all = "lowercase")]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    Elvish,
    Powershell,
}

/// Registration hook per shell, as documented by clap_complete's CompleteEnv.
const BASH_LINE: &str = "source <(COMPLETE=bash muster)";
const ZSH_LINE: &str = "source <(COMPLETE=zsh muster)";
const FISH_LINE: &str = "COMPLETE=fish muster | source";
const ELVISH_LINE: &str = "eval (E:COMPLETE=elvish muster | slurp)";
const POWERSHELL_LINE: &str = "$env:COMPLETE = \"powershell\"; muster | Out-String | Invoke-Expression; Remove-Item Env:\\COMPLETE";

/// Where each shell's hook line belongs.
const BASH_DESTINATION: &str = "~/.bashrc";
const ZSH_DESTINATION: &str = "~/.zshrc";
const FISH_DESTINATION: &str = "~/.config/fish/completions/muster.fish";
const ELVISH_DESTINATION: &str = "~/.elvish/rc.elv";
const POWERSHELL_DESTINATION: &str = "$PROFILE";

/// The shell hook line enabling muster's dynamic completions.
pub fn registration_line(shell: CompletionShell) -> &'static str {
    match shell {
        CompletionShell::Bash => BASH_LINE,
        CompletionShell::Zsh => ZSH_LINE,
        CompletionShell::Fish => FISH_LINE,
        CompletionShell::Elvish => ELVISH_LINE,
        CompletionShell::Powershell => POWERSHELL_LINE,
    }
}

/// The comment naming the destination file, then the hook line itself.
pub fn registration(shell: CompletionShell) -> [String; 2] {
    let destination = match shell {
        CompletionShell::Bash => BASH_DESTINATION,
        CompletionShell::Zsh => ZSH_DESTINATION,
        CompletionShell::Fish => FISH_DESTINATION,
        CompletionShell::Elvish => ELVISH_DESTINATION,
        CompletionShell::Powershell => POWERSHELL_DESTINATION,
    };
    [
        format!("# add to {destination}:"),
        registration_line(shell).to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each shell yields its documented dynamic-completion hook line.
    #[test]
    fn every_shell_has_its_registration_line() {
        assert_eq!(
            registration_line(CompletionShell::Bash),
            "source <(COMPLETE=bash muster)"
        );
        assert_eq!(
            registration_line(CompletionShell::Zsh),
            "source <(COMPLETE=zsh muster)"
        );
        assert_eq!(
            registration_line(CompletionShell::Fish),
            "COMPLETE=fish muster | source"
        );
        assert_eq!(
            registration_line(CompletionShell::Elvish),
            "eval (E:COMPLETE=elvish muster | slurp)"
        );
        assert!(registration_line(CompletionShell::Powershell).contains("COMPLETE"));
    }

    /// The printed output pairs a destination hint with the line.
    #[test]
    fn output_names_the_destination_file() {
        let [hint, line] = registration(CompletionShell::Zsh);
        assert!(hint.contains(".zshrc"));
        assert_eq!(line, registration_line(CompletionShell::Zsh));
    }
}

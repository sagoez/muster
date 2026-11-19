use std::path::{Path, PathBuf};

use clap::ValueEnum;
use strum::{Display, EnumIter, EnumMessage, EnumProperty};

/// The property naming the file a shell's hook line belongs in.
const DESTINATION: &str = "destination";
/// The property naming a shell's rc file under the home folder.
const HOME_RC: &str = "home_rc";
/// The property naming a shell's rc file under the XDG config folder.
const XDG_RC: &str = "xdg_rc";
/// The property naming a shell's completions drop-in under the XDG config
/// folder.
const XDG_DROP_IN: &str = "xdg_drop_in";

/// Shells supported by the dynamic completion engine, named as their
/// binaries are, so `$SHELL` reads as one. A shell's message is its hook
/// line, as documented by clap_complete's CompleteEnv; its props name where
/// that line goes and the files `muster doctor` looks for it in.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Display, EnumIter, EnumMessage, EnumProperty,
)]
#[strum(serialize_all = "kebab-case")]
pub enum CompletionShell {
    #[strum(
        message = "source <(COMPLETE=bash muster)",
        props(destination = "~/.bashrc", home_rc = ".bashrc")
    )]
    Bash,
    #[strum(
        message = "source <(COMPLETE=zsh muster)",
        props(destination = "~/.zshrc", home_rc = ".zshrc")
    )]
    Zsh,
    #[strum(
        message = "COMPLETE=fish muster | source",
        props(
            destination = "~/.config/fish/completions/muster.fish",
            xdg_rc = "fish/config.fish",
            xdg_drop_in = "fish/completions/muster.fish"
        )
    )]
    Fish,
    #[strum(
        message = "eval (E:COMPLETE=elvish muster | slurp)",
        props(
            destination = "~/.elvish/rc.elv",
            home_rc = ".elvish/rc.elv",
            xdg_rc = "elvish/rc.elv"
        )
    )]
    Elvish,
    #[value(alias = "pwsh")]
    #[strum(
        message = "$env:COMPLETE = \"powershell\"; muster | Out-String | Invoke-Expression; Remove-Item Env:\\COMPLETE",
        props(
            destination = "$PROFILE",
            xdg_rc = "powershell/Microsoft.PowerShell_profile.ps1"
        )
    )]
    Powershell,
}

impl CompletionShell {
    /// The shell's rc file under the XDG config folder, if it reads one.
    pub fn xdg_rc(self) -> Option<&'static str> {
        self.get_str(XDG_RC)
    }

    /// The shell's completions drop-in under the XDG config folder, if it
    /// takes one.
    pub fn xdg_drop_in(self) -> Option<&'static str> {
        self.get_str(XDG_DROP_IN)
    }

    /// The files that may hold this shell's hook line: its rc file under
    /// `home`, then its rc file and its completions drop-in under
    /// `xdg_config`, each where the shell reads one.
    pub fn rc_files(self, home: &Path, xdg_config: &Path) -> Vec<PathBuf> {
        [
            self.get_str(HOME_RC).map(|file| home.join(file)),
            self.xdg_rc().map(|file| xdg_config.join(file)),
            self.xdg_drop_in().map(|file| xdg_config.join(file)),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
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

    /// The home folder the rc-file tests read under.
    const HOME: &str = "/home/me";
    /// The XDG config folder the rc-file tests read under.
    const CONFIG: &str = "/home/me/.config";

    /// The rc files `shell` reads, under [`HOME`] and [`CONFIG`].
    fn files(shell: CompletionShell) -> Vec<PathBuf> {
        shell.rc_files(Path::new(HOME), Path::new(CONFIG))
    }

    /// Each shell's hook line is looked for in the rc files it reads, under
    /// home or the XDG config folder, as it always was.
    #[test]
    fn every_shell_names_the_rc_files_it_reads() {
        let home = Path::new(HOME);
        let config = Path::new(CONFIG);

        assert_eq!(files(CompletionShell::Bash), [home.join(".bashrc")]);
        assert_eq!(files(CompletionShell::Zsh), [home.join(".zshrc")]);
        assert_eq!(files(CompletionShell::Fish), [
            config.join("fish/config.fish"),
            config.join("fish/completions/muster.fish")
        ]);
        assert_eq!(files(CompletionShell::Elvish), [
            home.join(".elvish/rc.elv"),
            config.join("elvish/rc.elv")
        ]);
        assert_eq!(files(CompletionShell::Powershell), [
            config.join("powershell/Microsoft.PowerShell_profile.ps1")
        ]);
    }

    /// The printed output pairs a destination hint with the line.
    #[test]
    fn output_names_the_destination_file() {
        let [hint, line] = registration(CompletionShell::Zsh);
        assert!(hint.contains(".zshrc"));
        assert_eq!(Some(line.as_str()), CompletionShell::Zsh.get_message());
    }
}

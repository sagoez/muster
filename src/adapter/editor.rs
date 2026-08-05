use std::{
    path::Path,
    process::{Child, Command, Stdio},
};

use crate::domain::{
    port::{EditorError, EditorLauncher, SettingsStore},
    settings::EditorSettings,
};

/// Environment variable naming the user's preferred visual (GUI) editor.
const VISUAL_ENV: &str = "VISUAL";
/// Environment variable naming the user's fallback editor.
const EDITOR_ENV: &str = "EDITOR";
/// Editors that run inside a terminal and so need a terminal window opened for
/// them. When `editor.terminal` is unset, muster wraps one of these in
/// [`AUTO_TERMINAL_LAUNCHER`] so it opens its own window out of the box.
const TERMINAL_EDITORS: &[&str] = &[
    "nvim",
    "vim",
    "vi",
    "nvi",
    "view",
    "nano",
    "pico",
    "emacs",
    "emacsclient",
    "helix",
    "hx",
    "kak",
    "kakoune",
    "micro",
    "ne",
    "joe",
    "mg",
    "vis",
    "ed",
];
/// Default launcher used to open a terminal editor's window when no
/// `editor.terminal` is configured, chosen per platform: the freedesktop Default
/// Terminal spec launcher on Linux, Windows Terminal on Windows, and nothing on
/// platforms with no standard launcher (macOS, ...), where a terminal editor
/// needs an explicit `editor.terminal`. Overridable; if the launcher is not
/// installed the launch fails with a clear error rather than opening nothing.
#[cfg(target_os = "linux")]
const AUTO_TERMINAL_LAUNCHER: Option<&str> = Some("xdg-terminal-exec");
#[cfg(target_os = "windows")]
const AUTO_TERMINAL_LAUNCHER: Option<&str> = Some("wt");
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
const AUTO_TERMINAL_LAUNCHER: Option<&str> = None;

/// Opens a project in an editor launched as a detached window, so muster keeps
/// running untouched. The editor command comes from settings (`editor.command`,
/// falling back to `$VISUAL` then `$EDITOR`). A known terminal editor is opened in
/// its own window automatically via [`AUTO_TERMINAL_LAUNCHER`]; `editor.terminal`
/// overrides that with an explicit launcher, and a GUI editor launches directly.
/// Settings are read on each open, so an edit to `settings.yml` takes effect on
/// the next launch without restarting.
pub struct EnvEditorLauncher {
    settings: Box<dyn SettingsStore>,
}

impl EnvEditorLauncher {
    /// Builds a launcher that reads editor settings from `settings` at each open.
    pub fn new(settings: Box<dyn SettingsStore>) -> Self {
        Self { settings }
    }
}

impl EditorLauncher for EnvEditorLauncher {
    fn open(&self, directory: &Path) -> Result<(), EditorError> {
        // A load failure means settings.yml is malformed or unreadable, not absent
        // (an absent file materializes defaults); surface it rather than silently
        // falling back to a possibly different editor from the environment.
        let settings = self.settings.load()?;
        let mut argv = resolve_editor_argv(settings.editor())?;
        let program = argv.remove(0);
        launch(&program, &argv, directory)
    }
}

/// Resolves the editor argv (program plus arguments, before the project path):
/// the configured `command` (else `$VISUAL`, else `$EDITOR`), with the configured
/// `terminal` launcher prepended when set. An empty result or a command with
/// unbalanced quoting yields [`EditorError::NoEditor`].
fn resolve_editor_argv(editor: &EditorSettings) -> Result<Vec<String>, EditorError> {
    let command = Some(editor.command())
        .filter(|command| !command.trim().is_empty())
        .cloned()
        .or_else(|| non_empty_env(VISUAL_ENV))
        .or_else(|| non_empty_env(EDITOR_ENV))
        .ok_or(EditorError::NoEditor)?;
    // An explicit `editor.terminal` wins; otherwise a known terminal editor is
    // wrapped in the platform's default launcher so it opens its own window with
    // no config. A terminal editor on a platform with no default launcher and no
    // configured one is refused rather than launched with no usable tty.
    let terminal = match Some(editor.terminal())
        .filter(|terminal| !terminal.trim().is_empty())
        .cloned()
    {
        Some(terminal) => Some(terminal),
        None if command_needs_terminal(&command) => match AUTO_TERMINAL_LAUNCHER {
            Some(launcher) => Some(launcher.to_string()),
            None => return Err(EditorError::NoTerminalLauncher { editor: command }),
        },
        None => None,
    };
    let mut argv = Vec::new();
    if let Some(terminal) = terminal {
        argv.extend(parse_editor(&terminal).ok_or(EditorError::NoEditor)?);
    }
    argv.extend(parse_editor(&command).ok_or(EditorError::NoEditor)?);
    if argv.is_empty() {
        return Err(EditorError::NoEditor);
    }
    Ok(argv)
}

/// Whether the editor `command` names a terminal editor, so muster opens a
/// terminal window for it rather than launching it directly.
fn command_needs_terminal(command: &str) -> bool {
    let Some(first) = parse_editor(command).and_then(|parts| parts.into_iter().next()) else {
        return false;
    };
    let program = first.rsplit(['/', '\\']).next().unwrap_or(&first);
    // Windows executable names and extensions are case-insensitive, so match them
    // that way (and against the `.cmd`/`.bat` launcher shims) or `NVIM.EXE` would
    // be treated as a GUI editor and launched with no usable window.
    #[cfg(windows)]
    {
        let lowered = program.to_ascii_lowercase();
        let program = lowered
            .strip_suffix(".exe")
            .or_else(|| lowered.strip_suffix(".cmd"))
            .or_else(|| lowered.strip_suffix(".bat"))
            .unwrap_or(&lowered);
        TERMINAL_EDITORS.contains(&program)
    }
    #[cfg(not(windows))]
    {
        let program = program.strip_suffix(".exe").unwrap_or(program);
        TERMINAL_EDITORS.contains(&program)
    }
}

/// Spawns `program` with `args` and the project path detached from muster's
/// terminal, returning immediately. A failure to start is a
/// [`EditorError::Spawn`]; a started editor is not waited on.
fn launch(program: &str, args: &[String], directory: &Path) -> Result<(), EditorError> {
    let mut command = Command::new(program);
    command
        .args(args)
        .arg(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach(&mut command);
    let child = command.spawn().map_err(|source| EditorError::Spawn {
        editor: program.to_string(),
        source,
    })?;
    reap(child);
    Ok(())
}

/// Places the child in its own process group so muster's terminal signals do not
/// reach it and it outlives muster.
#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

/// Detaches the child from muster's Windows console and control group, so a later
/// Ctrl+C, Ctrl+Break, or console-close event in muster's console cannot terminate
/// it and it outlives muster. Null streams alone do not do this: a console child
/// stays attached to the parent console and its signals. `DETACHED_PROCESS` gives
/// the child no inherited console; `CREATE_NEW_PROCESS_GROUP` roots it in its own
/// group so console control signals are not propagated to it. A terminal editor is
/// wrapped in a launcher that opens its own window, so removing the inherited
/// console leaves it with a usable one rather than none.
#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    /// Creation flag: the child inherits no console from muster.
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    /// Creation flag: the child is the root of a new process group, so console
    /// control signals sent to muster's group are not delivered to it.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

/// Waits on a detached editor on a background thread so it is reaped when it
/// exits, without blocking the runtime loop.
fn reap(mut child: Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

/// The value of environment variable `name`, if set and not blank after trimming.
fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// Splits an editor command into program and arguments using POSIX shell quoting.
/// `None` only on an unbalanced quote.
#[cfg(not(windows))]
fn parse_editor(editor: &str) -> Option<Vec<String>> {
    shlex::split(editor)
}

/// Splits an editor command on Windows, where POSIX escaping would corrupt a
/// backslash path such as `C:\vim\vim.exe`. Whitespace separates arguments,
/// double quotes group spans, and backslashes stay literal.
#[cfg(windows)]
fn parse_editor(editor: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut has_token = false;
    for ch in editor.chars() {
        if ch == '"' {
            quoted = !quoted;
            has_token = true;
        } else if ch.is_whitespace() && !quoted {
            if has_token {
                args.push(std::mem::take(&mut current));
                has_token = false;
            }
        } else {
            current.push(ch);
            has_token = true;
        }
    }
    // An unmatched double quote is malformed; refuse it rather than launch an
    // unintended program or argument list.
    if quoted {
        return None;
    }
    if has_token {
        args.push(current);
    }
    Some(args)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{EnvEditorLauncher, launch, resolve_editor_argv};
    use crate::domain::{
        config::ConfigError,
        port::{EditorError, EditorLauncher, SettingsStore},
        settings::{EditorSettings, Settings},
    };

    fn editor(command: &str, terminal: &str) -> EditorSettings {
        EditorSettings::builder()
            .command(command.to_string())
            .terminal(terminal.to_string())
            .build()
    }

    /// A settings store whose load always fails, standing in for a malformed or
    /// unreadable `settings.yml`.
    struct FailingSettingsStore;

    impl SettingsStore for FailingSettingsStore {
        fn load(&self) -> Result<Settings, ConfigError> {
            Err(ConfigError::NoConfigDir)
        }

        fn save(&self, _settings: &Settings) -> Result<(), ConfigError> {
            Err(ConfigError::NoConfigDir)
        }
    }

    /// A settings load failure is surfaced as an editor error rather than silently
    /// falling back to `$VISUAL`/`$EDITOR`, so a broken config does not launch a
    /// different editor than the one the user configured.
    #[test]
    fn a_settings_load_failure_is_surfaced_not_swallowed() {
        let launcher = EnvEditorLauncher::new(Box::new(FailingSettingsStore));

        assert!(matches!(
            launcher.open(Path::new("/")),
            Err(EditorError::Settings { .. })
        ));
    }

    /// A terminal editor is wrapped in its configured launcher so it opens its own
    /// window, with the editor and (later) the path following.
    #[test]
    fn wraps_a_terminal_editor_in_its_launcher() {
        let settings = editor("nvim", "kitty");

        let argv = resolve_editor_argv(&settings).unwrap();

        assert_eq!(argv, vec!["kitty", "nvim"]);
    }

    /// A multi-word terminal launcher keeps its own arguments (e.g. `-e`).
    #[test]
    fn keeps_terminal_launcher_arguments() {
        let settings = editor("nvim", "alacritty -e");

        let argv = resolve_editor_argv(&settings).unwrap();

        assert_eq!(argv, vec!["alacritty", "-e", "nvim"]);
    }

    /// A GUI editor with no terminal launcher runs directly.
    #[test]
    fn a_gui_editor_launches_directly() {
        let settings = editor("code --new-window", "");

        let argv = resolve_editor_argv(&settings).unwrap();

        assert_eq!(argv, vec!["code", "--new-window"]);
    }

    /// A command with an unbalanced quote is malformed and refused rather than run
    /// as an unintended program, on both the POSIX and Windows parsers.
    #[test]
    fn an_unbalanced_quote_is_rejected() {
        let settings = editor("nvim \"unterminated", "");

        assert!(matches!(
            resolve_editor_argv(&settings),
            Err(EditorError::NoEditor)
        ));
    }

    /// A known terminal editor with no configured launcher is wrapped in the
    /// platform default launcher, so it opens its own window out of the box.
    /// Asserted on Linux, whose default launcher is `xdg-terminal-exec`.
    #[cfg(target_os = "linux")]
    #[test]
    fn auto_wraps_a_known_terminal_editor() {
        let settings = editor("nvim", "");

        let argv = resolve_editor_argv(&settings).unwrap();

        assert_eq!(argv, vec!["xdg-terminal-exec", "nvim"]);
    }

    /// The auto-wrap sees through a path and matches the executable name.
    #[cfg(target_os = "linux")]
    #[test]
    fn auto_wraps_a_path_qualified_terminal_editor() {
        let settings = editor("/usr/bin/nvim --clean", "");

        let argv = resolve_editor_argv(&settings).unwrap();

        assert_eq!(argv, vec!["xdg-terminal-exec", "/usr/bin/nvim", "--clean"]);
    }

    /// A launchable command spawns detached and returns at once (no blocking wait),
    /// so the runtime loop is never held while an editor is open.
    #[cfg(unix)]
    #[test]
    fn launching_a_valid_command_returns_without_waiting() {
        assert!(launch("true", &[], Path::new("/")).is_ok());
    }

    /// A command that cannot start is a spawn error rather than a silent success.
    #[test]
    fn launching_a_missing_command_is_a_spawn_error() {
        assert!(matches!(
            launch("muster-nonexistent-editor-command", &[], Path::new("/")),
            Err(EditorError::Spawn { .. })
        ));
    }
}

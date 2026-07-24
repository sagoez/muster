use std::{
    env,
    io::Write,
    process::{Command, Stdio},
};

/// A clipboard-writing helper program and its arguments.
struct ClipboardCommand {
    program: &'static str,
    args: &'static [&'static str],
}

/// Writes `text` through a native clipboard tool, herdr's preferred path.
/// Returns whether any tool accepted it; remote (SSH) sessions skip straight
/// to `false` because only the host terminal's OSC 52 reaches the local
/// clipboard there.
pub fn write_native(text: &str) -> bool {
    if prefers_osc52() {
        return false;
    }
    clipboard_commands()
        .iter()
        .any(|command| run_clipboard_command(command, text))
}

/// Whether the session should rely on OSC 52 instead of local tools.
fn prefers_osc52() -> bool {
    env::var_os("SSH_CONNECTION").is_some() || env::var_os("SSH_TTY").is_some()
}

/// The platform clipboard writers worth trying, in herdr's order.
#[cfg(target_os = "macos")]
fn clipboard_commands() -> Vec<ClipboardCommand> {
    vec![ClipboardCommand {
        program: "pbcopy",
        args: &[],
    }]
}

/// The platform clipboard writers worth trying, in herdr's order.
#[cfg(all(unix, not(target_os = "macos")))]
fn clipboard_commands() -> Vec<ClipboardCommand> {
    let mut commands = Vec::new();
    if env::var_os("WAYLAND_DISPLAY").is_some() {
        commands.push(ClipboardCommand {
            program: "wl-copy",
            args: &["--type", "text/plain;charset=utf-8"],
        });
    }
    if env::var_os("DISPLAY").is_some() {
        commands.push(ClipboardCommand {
            program: "xclip",
            args: &["-selection", "clipboard", "-in"],
        });
        commands.push(ClipboardCommand {
            program: "xsel",
            args: &["--clipboard", "--input"],
        });
    }
    commands
}

/// The platform clipboard writers worth trying; none on other targets, where
/// OSC 52 is the only route.
#[cfg(not(unix))]
fn clipboard_commands() -> Vec<ClipboardCommand> {
    Vec::new()
}

/// Pipes `text` into one clipboard helper, reporting whether it succeeded.
fn run_clipboard_command(command: &ClipboardCommand, text: &str) -> bool {
    let Ok(mut child) = Command::new(command.program)
        .args(command.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let written = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
    let exited = child.wait().map(|status| status.success()).unwrap_or(false);
    written && exited
}

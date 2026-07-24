use std::{
    env,
    ffi::OsStr,
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
pub fn prefers_osc52() -> bool {
    env::var_os("SSH_CONNECTION").is_some() || env::var_os("SSH_TTY").is_some()
}

/// The native clipboard tool a copy would try first, if any is found on PATH.
pub fn preferred_tool() -> Option<&'static str> {
    clipboard_commands()
        .iter()
        .find(|command| on_path(command.program))
        .map(|command| command.program)
}

/// Returns true when `program` exists as an executable regular file on PATH.
fn on_path(program: &str) -> bool {
    // `var_os`, not `var`: a Unix PATH may validly contain non-UTF-8 entries
    // and must not disqualify every tool on it.
    let Some(path_var) = env::var_os("PATH") else {
        return false;
    };
    on_path_in(program, &path_var)
}

/// Unix permission bits that indicate at least one executable bit is set.
#[cfg(unix)]
const EXECUTABLE_BITS: u32 = 0o111;

/// Returns true when `program` is an executable regular file somewhere in `path`.
fn on_path_in(program: &str, path: &OsStr) -> bool {
    env::split_paths(path).any(|dir| {
        let candidate = dir.join(program);
        let Ok(meta) = std::fs::metadata(&candidate) else {
            return false;
        };
        if !meta.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            meta.permissions().mode() & EXECUTABLE_BITS != 0
        }
        #[cfg(not(unix))]
        {
            true
        }
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A regular file without any executable bit, for the rejection case.
    #[cfg(unix)]
    const NON_EXECUTABLE_MODE: u32 = 0o644;
    /// A regular executable file, for the acceptance case.
    #[cfg(unix)]
    const EXECUTABLE_MODE: u32 = 0o755;

    /// `preferred_tool` only reports a tool that is actually on PATH (or None).
    #[test]
    fn preferred_tool_only_reports_a_tool_that_exists_on_path() {
        match preferred_tool() {
            Some(tool) => assert!(on_path(tool), "{tool} was reported but is not on PATH"),
            None => {
                // Either no candidate commands, or none are on PATH - both valid.
                for command in clipboard_commands() {
                    assert!(!on_path(command.program));
                }
            },
        }
    }

    /// A non-UTF-8 PATH entry does not disqualify tools elsewhere on PATH.
    #[cfg(unix)]
    #[test]
    fn on_path_in_accepts_a_non_utf8_path() {
        use std::{
            ffi::OsString,
            fs,
            os::unix::{ffi::OsStringExt, fs::PermissionsExt},
        };

        let dir = std::env::temp_dir().join(format!("muster-clip-utf8-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let tool = dir.join("muster-fake-tool");
        fs::write(&tool, "").unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(EXECUTABLE_MODE)).unwrap();
        // PATH = <non-utf8 dir>:<real dir>
        let mut path_var = OsString::from_vec(vec![b'/', b't', b'm', b'p', b'/', 0xFF]);
        path_var.push(":");
        path_var.push(&dir);

        assert!(on_path_in("muster-fake-tool", &path_var));
        fs::remove_dir_all(dir).unwrap();
    }

    /// A non-executable file with the right name must not count as a candidate.
    #[cfg(unix)]
    #[test]
    fn on_path_in_rejects_non_executable_files() {
        use std::{ffi::OsString, fs, os::unix::fs::PermissionsExt};

        let dir = std::env::temp_dir().join(format!("muster-clipboard-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();

        let program = "fake-clipboard-tool";
        let path = dir.join(program);
        fs::write(&path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(NON_EXECUTABLE_MODE)).unwrap();

        let path_var: OsString = dir.as_os_str().to_os_string();
        assert!(
            !on_path_in(program, &path_var),
            "non-executable file must not match"
        );

        // Make it executable - now it should match.
        fs::set_permissions(&path, fs::Permissions::from_mode(EXECUTABLE_MODE)).unwrap();
        assert!(on_path_in(program, &path_var), "executable file must match");

        fs::remove_dir_all(dir).unwrap();
    }
}

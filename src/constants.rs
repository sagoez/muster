/// Human-facing application name, reused for window titles and labels.
pub const APP_NAME: &str = "muster";
/// This build's version, which a client and the service compare so a newer
/// binary replaces an older service.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The file argument that means stdout or stdin.
pub const STDIO_PATH: &str = "-";

/// Environment variable exported into each process, holding its project's
/// folder so the `muster` CLI run inside it targets that project.
pub const MUSTER_PROJECT_ENV: &str = "MUSTER_PROJECT";

/// Environment variable the shell hook exports, holding the hooked shell's
/// pid, so `muster doctor` can tell whether its parent shell runs the hook.
pub const MUSTER_SHELL_ENV: &str = "MUSTER_SHELL_PID";

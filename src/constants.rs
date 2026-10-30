/// Human-facing application name, reused for window titles and labels.
pub const APP_NAME: &str = "muster";
/// This build's version, which a client and the service compare so a newer
/// binary replaces an older service.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The variable listing the folders a binary is looked up in.
pub const PATH_ENV: &str = "PATH";
/// The variable naming the XDG config root.
pub const XDG_CONFIG_HOME_ENV: &str = "XDG_CONFIG_HOME";
/// The config root under home where `XDG_CONFIG_HOME` does not move it.
pub const DOT_CONFIG: &str = ".config";

/// The file argument that means stdout or stdin.
pub const STDIO_PATH: &str = "-";

/// Environment variable exported into each process, holding its project's
/// folder so the `muster` CLI run inside it targets that project.
pub const MUSTER_PROJECT_ENV: &str = "MUSTER_PROJECT";

/// Environment variable the shell hook exports, holding the hooked shell's
/// pid, so `muster doctor` can tell whether its parent shell runs the hook.
pub const MUSTER_SHELL_ENV: &str = "MUSTER_SHELL_PID";

/// Prefix of the variables each word of an `open` pane's command travels in.
pub const MUSTER_OPEN_ARG_PREFIX: &str = "MUSTER_OPEN_ARG_";
/// The first of those, holding the program: set only in a pane `open` made,
/// whose shell the hook leaves unreported.
pub const MUSTER_OPEN_PROGRAM_ENV: &str = "MUSTER_OPEN_ARG_0";

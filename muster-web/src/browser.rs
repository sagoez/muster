use std::{
    env,
    ffi::OsString,
    io,
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    thread,
};

/// The variable naming the browser to open pages in, which wins over the
/// platform's opener.
const BROWSER_ENV: &str = "BROWSER";
/// What opens a URL in the default browser on this platform.
#[cfg(target_os = "macos")]
const PLATFORM_OPENER: &str = "open";
/// What opens a URL in the default browser on this platform.
#[cfg(not(target_os = "macos"))]
const PLATFORM_OPENER: &str = "xdg-open";
/// The name of the thread that reaps the opener once it exits.
const REAPER_THREAD: &str = "view-browser-reaper";
/// The process group a spawned command leads: one of its own.
const OWN_PROCESS_GROUP: i32 = 0;

/// Opens `url` in `$BROWSER`, else in the platform's default browser. The
/// command runs in a process group of its own, so the Ctrl-C or hangup that
/// ends the view, sent to the view's whole group, never reaches a browser
/// it started; its output goes nowhere and it is reaped once it exits.
///
/// # Errors
/// Returns the failure to spawn the command.
pub fn open_browser(url: &str) -> io::Result<()> {
    let program = env::var_os(BROWSER_ENV)
        .filter(|browser| !browser.is_empty())
        .unwrap_or_else(|| OsString::from(PLATFORM_OPENER));
    let mut child = Command::new(program)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(OWN_PROCESS_GROUP)
        .spawn()?;
    // A thread that cannot start leaves the opener unreaped, which costs a
    // zombie until the view exits and nothing else.
    let _ = thread::Builder::new()
        .name(REAPER_THREAD.to_string())
        .spawn(move || child.wait());
    Ok(())
}

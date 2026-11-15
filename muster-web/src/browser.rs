use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, ErrorKind},
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    thread,
};

/// The variable naming the browser to open pages in, which wins over the
/// platform's openers.
const BROWSER_ENV: &str = "BROWSER";
/// What opens a URL in the default browser on this platform.
#[cfg(target_os = "macos")]
const PLATFORM_OPENERS: &[&str] = &["open"];
/// What opens a URL in the default browser on this platform, in the order
/// they are tried: the desktop's opener, WSL's, then Debian's alternatives.
#[cfg(not(target_os = "macos"))]
const PLATFORM_OPENERS: &[&str] = &["xdg-open", "wslview", "sensible-browser", "x-www-browser"];
/// Why no browser opened when no opener could be found.
const NO_OPENER: &str = "no browser opener was found; set $BROWSER to one";
/// The name of the thread that reaps the opener once it exits.
const REAPER_THREAD: &str = "view-browser-reaper";
/// The process group a spawned command leads: one of its own.
const OWN_PROCESS_GROUP: i32 = 0;

/// Opens `url` in `$BROWSER`, else in the first of the platform's openers
/// that is there. The command runs in a process group of its own, so the
/// Ctrl-C or hangup that ends the view, sent to the view's whole group,
/// never reaches a browser it started; its output goes nowhere and it is
/// reaped once it exits.
///
/// # Errors
/// Returns why no opener started: none was found, or one that was found
/// could not be run.
pub fn open_browser(url: &str) -> io::Result<()> {
    let browser = env::var_os(BROWSER_ENV).filter(|browser| !browser.is_empty());
    let openers = browser
        .into_iter()
        .chain(PLATFORM_OPENERS.iter().map(OsString::from));
    open_with(openers, url)
}

/// Opens `url` with the first of `openers` that is there: one that is not
/// found moves on to the next.
///
/// # Errors
/// Returns [`NO_OPENER`] when none was found, or the failure to run the
/// first that was found.
fn open_with(openers: impl IntoIterator<Item = OsString>, url: &str) -> io::Result<()> {
    for opener in openers {
        match spawn(&opener, url) {
            Err(error) if error.kind() == ErrorKind::NotFound => {},
            started => return started,
        }
    }
    Err(io::Error::new(ErrorKind::NotFound, NO_OPENER))
}

/// Starts `opener` on `url` in a process group of its own, its output going
/// nowhere, and reaps it once it exits.
///
/// # Errors
/// Returns the failure to spawn it.
fn spawn(opener: &OsStr, url: &str) -> io::Result<()> {
    let mut child = Command::new(opener)
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

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, Permissions},
        os::unix::fs::PermissionsExt,
        path::Path,
    };

    use muster_os::{test_scratch::short_scratch, test_wait::wait_until};

    use super::*;

    /// The address the tests open.
    const URL: &str = "http://127.0.0.1:1/open/code";
    /// The mode that lets the stand-in opener run.
    const EXECUTABLE: u32 = 0o755;

    /// A stand-in opener at `path` that writes the URL it is given to
    /// `opened`.
    fn stand_in(path: &Path, opened: &Path) {
        fs::write(
            path,
            format!("#!/bin/sh\nprintf %s \"$1\" > '{}'\n", opened.display()),
        )
        .unwrap();
        fs::set_permissions(path, Permissions::from_mode(EXECUTABLE)).unwrap();
    }

    /// An opener that is not there moves on to the next, which opens the
    /// URL.
    #[test]
    fn a_missing_opener_moves_on_to_the_next() {
        let scratch = short_scratch("mv-");
        let (opener, opened) = (scratch.path().join("opener"), scratch.path().join("url"));
        stand_in(&opener, &opened);
        let missing = scratch.path().join("missing");

        open_with([missing.into_os_string(), opener.into_os_string()], URL).unwrap();

        wait_until("the opener to write the URL", || {
            fs::read_to_string(&opened).is_ok_and(|url| url == URL)
        });
    }

    /// With no opener there, the failure says none was found.
    #[test]
    fn no_opener_found_says_so() {
        let scratch = short_scratch("mv-");
        let missing = || scratch.path().join("missing").into_os_string();

        let error = open_with([missing(), missing()], URL).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::NotFound);
        assert_eq!(error.to_string(), NO_OPENER);
    }

    /// An opener that is there but cannot run is reported, not passed over.
    #[test]
    fn an_opener_that_cannot_run_is_reported() {
        let scratch = short_scratch("mv-");
        let (opener, opened) = (scratch.path().join("opener"), scratch.path().join("url"));
        stand_in(&opener, &opened);
        let unrunnable = scratch.path().join("unrunnable");
        fs::write(&unrunnable, "").unwrap();

        let error =
            open_with([unrunnable.into_os_string(), opener.into_os_string()], URL).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert!(!opened.exists());
    }
}

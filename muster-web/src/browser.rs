use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, ErrorKind},
    os::unix::{ffi::OsStrExt, process::CommandExt},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::Duration,
};

use crossbeam_channel::bounded;

/// The variable naming the browser to open pages in: when set, it is the
/// only one tried, since the user chose it.
const BROWSER_ENV: &str = "BROWSER";
/// What joins the chosen browser's name to why it could not be run.
const CHOSEN_UNRUNNABLE: &str = " (from $BROWSER) could not be run: ";
/// What opens a URL in the default browser on this platform.
#[cfg(target_os = "macos")]
const PLATFORM_OPENERS: &[&str] = &["open"];
/// What opens a URL in the default browser on this platform, in the order
/// they are tried: the desktop's opener, WSL's, then Debian's alternatives.
#[cfg(not(target_os = "macos"))]
const PLATFORM_OPENERS: &[&str] = &["xdg-open", "wslview", "sensible-browser", "x-www-browser"];
/// Why no browser opened when no opener could be found.
const NO_OPENER: &str = "no browser opener was found; set $BROWSER to one";
/// What joins an opener's name to why it could not open the page.
const OPENER_FAILED: &str = " could not open the page: ";
/// The name of the thread that reaps the opener once it exits.
const REAPER_THREAD: &str = "view-browser-reaper";
/// The process group a spawned command leads: one of its own.
const OWN_PROCESS_GROUP: i32 = 0;
/// How long an opener is given to fail: one that exits unsuccessfully
/// within it, as `xdg-open` does with no handler, is passed over, and one
/// still running after it, as a browser itself is, counts as started.
const OPENER_GRACE: Duration = Duration::from_secs(1);
/// What separates the folders of a program path, which a bare name lacks.
const PATH_SEPARATOR: u8 = b'/';

/// Opens `url` in `$BROWSER`, taken as one program and the only one tried
/// when it is set, else in the first of the platform's openers that is
/// there and does not fail. The command runs in a process group of its
/// own, so the Ctrl-C or hangup that ends the view, sent to the view's
/// whole group, never reaches a browser it started; its output goes
/// nowhere and it is reaped once it exits.
///
/// # Errors
/// Returns why no browser started, as [`open_from`] does.
pub fn open_browser(url: &str) -> io::Result<()> {
    open_from(
        env::var_os(BROWSER_ENV),
        PLATFORM_OPENERS.iter().map(OsString::from),
        url,
    )
}

/// Opens `url` with `browser`, the value of `$BROWSER`, when it is set and
/// not empty, trying nothing else; else with the first of `platform` that
/// works, as [`open_with`] does.
///
/// # Errors
/// Returns why the chosen browser failed or could not be run, or, with
/// none chosen, why no opener started.
fn open_from(
    browser: Option<OsString>,
    platform: impl IntoIterator<Item = OsString>,
    url: &str,
) -> io::Result<()> {
    match browser.filter(|browser| !browser.is_empty()) {
        Some(browser) => open_chosen(&browser, url),
        None => open_with(platform, url),
    }
}

/// Opens `url` with `browser`, the one the user chose: when it fails
/// within [`OPENER_GRACE`] or cannot be run, nothing else is tried.
///
/// # Errors
/// Returns its failure, or why it could not be run.
fn open_chosen(browser: &OsStr, url: &str) -> io::Result<()> {
    match spawn(browser, url) {
        Ok(None) => Ok(()),
        Ok(Some(status)) => Err(opener_failed(browser, status)),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!("{}{CHOSEN_UNRUNNABLE}{error}", browser.to_string_lossy()),
        )),
    }
}

/// Why `opener` could not open the page: it exited with `status`.
fn opener_failed(opener: &OsStr, status: ExitStatus) -> io::Error {
    io::Error::other(format!(
        "{}{OPENER_FAILED}{status}",
        opener.to_string_lossy()
    ))
}

/// Opens `url` with the first of `openers` that is there and does not fail
/// within [`OPENER_GRACE`]: one that is [passed over](passed_over), or that
/// exits unsuccessfully in that time, moves on to the next. It waits on the
/// caller's thread.
///
/// # Errors
/// Returns [`NO_OPENER`] when none was found, the failure of the last that
/// exited unsuccessfully, or the failure to run one named by its path.
fn open_with(openers: impl IntoIterator<Item = OsString>, url: &str) -> io::Result<()> {
    let mut failed = None;
    for opener in openers {
        match spawn(&opener, url) {
            Ok(None) => return Ok(()),
            Ok(Some(status)) => failed = Some(opener_failed(&opener, status)),
            Err(error) if passed_over(&opener, &error) => {},
            Err(error) => return Err(error),
        }
    }
    Err(failed.unwrap_or_else(|| io::Error::new(ErrorKind::NotFound, NO_OPENER)))
}

/// Whether `opener` failing to start with `error` moves on to the next: it
/// is not there, or it is a bare name the search of `PATH` found only in a
/// folder it may not run from. One named by its path that cannot run is
/// reported instead.
fn passed_over(opener: &OsStr, error: &io::Error) -> bool {
    let bare = !opener.as_bytes().contains(&PATH_SEPARATOR);
    match error.kind() {
        ErrorKind::NotFound => true,
        ErrorKind::PermissionDenied => bare,
        _ => false,
    }
}

/// Starts `opener` on `url` in a process group of its own, its output going
/// nowhere, reaps it once it exits, and waits up to [`OPENER_GRACE`] for
/// that: the status it exited unsuccessfully with in that time, none when
/// it succeeded or still runs.
///
/// # Errors
/// Returns the failure to spawn it.
fn spawn(opener: &OsStr, url: &str) -> io::Result<Option<ExitStatus>> {
    let mut child = Command::new(opener)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(OWN_PROCESS_GROUP)
        .spawn()?;
    let (exited, exit) = bounded(1);
    // A thread that cannot start leaves the opener unreaped, which costs a
    // zombie until the view exits, and its exit unknown, so it counts as
    // started.
    let reaper = thread::Builder::new()
        .name(REAPER_THREAD.to_string())
        .spawn(move || {
            let _ = exited.send(child.wait());
        });
    if reaper.is_err() {
        return Ok(None);
    }
    Ok(match exit.recv_timeout(OPENER_GRACE) {
        Ok(Ok(status)) if !status.success() => Some(status),
        _ => None,
    })
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
    /// The code `xdg-open` exits with when nothing handles the page.
    const NO_HANDLER: u8 = 3;

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

    /// A stand-in opener at `path` that fails at once with `code`, as
    /// `xdg-open` does with no handler for the page.
    fn failing(path: &Path, code: u8) {
        fs::write(path, format!("#!/bin/sh\nexit {code}\n")).unwrap();
        fs::set_permissions(path, Permissions::from_mode(EXECUTABLE)).unwrap();
    }

    /// An opener that starts and then fails moves on to the next, which
    /// opens the URL; with none after it, its failure is reported.
    #[test]
    fn an_opener_that_fails_moves_on_to_the_next() {
        let scratch = short_scratch("mv-");
        let (opener, opened) = (scratch.path().join("opener"), scratch.path().join("url"));
        stand_in(&opener, &opened);
        let failed = scratch.path().join("failed");
        failing(&failed, NO_HANDLER);

        open_with(
            [failed.clone().into_os_string(), opener.into_os_string()],
            URL,
        )
        .unwrap();
        let alone = open_with([failed.clone().into_os_string()], URL).unwrap_err();

        wait_until("the opener to write the URL", || {
            fs::read_to_string(&opened).is_ok_and(|url| url == URL)
        });
        let reported = alone.to_string();
        assert!(
            reported.starts_with(&*failed.to_string_lossy()),
            "{reported}"
        );
        assert!(reported.contains(&NO_HANDLER.to_string()), "{reported}");
    }

    /// A bare name the search of `PATH` cannot run is passed over like one
    /// it cannot find, while one named by its path is reported.
    #[test]
    fn only_a_bare_opener_that_cannot_run_is_passed_over() {
        let denied = || io::Error::from(ErrorKind::PermissionDenied);
        let missing = || io::Error::from(ErrorKind::NotFound);

        assert!(passed_over(OsStr::new("xdg-open"), &denied()));
        assert!(passed_over(OsStr::new("xdg-open"), &missing()));
        assert!(!passed_over(OsStr::new("/opt/bin/browser"), &denied()));
        assert!(passed_over(OsStr::new("/opt/bin/browser"), &missing()));
        assert!(!passed_over(
            OsStr::new("xdg-open"),
            &io::Error::from(ErrorKind::OutOfMemory)
        ));
    }

    /// A `$BROWSER` that fails stops the chain: nothing after it opens the
    /// page, and its failure is reported.
    #[test]
    fn a_failing_browser_opens_nothing_else() {
        let scratch = short_scratch("mv-");
        let (opener, opened) = (scratch.path().join("opener"), scratch.path().join("url"));
        stand_in(&opener, &opened);

        let error = open_from(
            Some(OsString::from("false")),
            [opener.into_os_string()],
            URL,
        )
        .unwrap_err();

        assert!(error.to_string().starts_with("false"), "{error}");
        assert!(!opened.exists());
    }

    /// A `$BROWSER` that is not there is reported as the chosen browser,
    /// and nothing after it is tried.
    #[test]
    fn a_missing_browser_opens_nothing_else() {
        let scratch = short_scratch("mv-");
        let (opener, opened) = (scratch.path().join("opener"), scratch.path().join("url"));
        stand_in(&opener, &opened);
        let missing = scratch.path().join("missing");

        let error = open_from(
            Some(missing.into_os_string()),
            [opener.into_os_string()],
            URL,
        )
        .unwrap_err();

        assert_eq!(error.kind(), ErrorKind::NotFound);
        assert!(error.to_string().contains(CHOSEN_UNRUNNABLE), "{error}");
        assert!(!opened.exists());
    }

    /// An empty `$BROWSER` counts as unset: the platform's openers are
    /// tried.
    #[test]
    fn an_empty_browser_falls_back_to_the_platform() {
        let scratch = short_scratch("mv-");
        let (opener, opened) = (scratch.path().join("opener"), scratch.path().join("url"));
        stand_in(&opener, &opened);

        open_from(Some(OsString::new()), [opener.into_os_string()], URL).unwrap();

        wait_until("the opener to write the URL", || {
            fs::read_to_string(&opened).is_ok_and(|url| url == URL)
        });
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

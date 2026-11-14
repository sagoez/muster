use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use muster_domain::value::LocalPort;
use muster_os::private_dir::prepare_state_dir;
use opener::OpenError;

use super::web_error::WebError;

/// What every launch page's file name starts with, before the view's port.
const PAGE_PREFIX: &str = "view-";
/// The launch page's file extension.
const PAGE_EXTENSION: &str = ".html";
/// Only the user may read the page: it holds the token.
const PAGE_MODE: u32 = 0o600;
/// The page up to the address it sends the browser on to. A meta refresh,
/// so the page runs no script.
const REDIRECT_HEAD: &str = "<!doctype html>\n<meta charset=\"utf-8\">\n<title>muster view</title>\n<meta http-equiv=\"refresh\" content=\"0; url=";
/// The page after that address.
const REDIRECT_TAIL: &str = "\">\n";

/// A private page in muster's state directory that sends the browser on to
/// the view's tokened address. The browser is handed this file's path, so
/// the token never appears in a process's arguments, which every local user
/// can read. The file is removed when the page is dropped.
pub struct LaunchPage {
    /// The page's file.
    path: PathBuf,
}

impl LaunchPage {
    /// Writes the page for the view on `port` at `url` into the private
    /// directory `dir`, then hands its path to `open`. A page that could not
    /// be opened is removed again.
    ///
    /// # Errors
    /// Returns [`WebError::Page`] when the page cannot be written, or
    /// [`WebError::Browser`] when `open` fails.
    pub fn open(
        dir: &Path,
        port: LocalPort,
        url: &str,
        open: impl FnOnce(&Path) -> Result<(), OpenError>,
    ) -> Result<Self, WebError> {
        let page = Self::write(dir, port, url).map_err(WebError::Page)?;
        open(&page.path).map_err(WebError::Browser)?;
        Ok(page)
    }

    /// Writes the page owner-only into `dir`, made private first, replacing
    /// one a killed view left behind.
    ///
    /// # Errors
    /// Returns the failure to prepare the directory or to create or write
    /// the file.
    fn write(dir: &Path, port: LocalPort, url: &str) -> io::Result<Self> {
        prepare_state_dir(dir)?;
        let path = dir.join(format!("{PAGE_PREFIX}{port}{PAGE_EXTENSION}"));
        match fs::remove_file(&path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {},
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(PAGE_MODE)
            .open(&path)?;
        let page = Self { path };
        file.write_all(format!("{REDIRECT_HEAD}{url}{REDIRECT_TAIL}").as_bytes())?;
        Ok(page)
    }
}

impl Drop for LaunchPage {
    /// Removes the page, and with it the token it holds.
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, os::unix::fs::PermissionsExt};

    use tempfile::TempDir;

    use super::*;

    /// The port the pages under test name.
    const PORT: u16 = 4173;
    /// A tokened address as the view prints it.
    const URL: &str = "http://127.0.0.1:4173/v/0123456789abcdef/";
    /// The token in [`URL`].
    const TOKEN: &str = "0123456789abcdef";

    /// The opener is handed the page's path, never the tokened address; the
    /// page is owner-only, sends the browser on to the address, and is gone
    /// once dropped.
    #[test]
    fn the_browser_is_handed_a_private_page_not_the_token() {
        let dir = TempDir::new().unwrap();
        let handed = RefCell::new(None);

        let page = LaunchPage::open(dir.path(), LocalPort::try_new(PORT).unwrap(), URL, |path| {
            let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
            let text = fs::read_to_string(path).unwrap();
            handed.replace(Some((path.to_path_buf(), mode, text)));
            Ok(())
        })
        .unwrap();

        let (path, mode, text) = handed.take().unwrap();
        assert_eq!(path, dir.path().join("view-4173.html"));
        assert!(!path.to_string_lossy().contains(TOKEN));
        assert_eq!(mode, 0o600);
        assert!(
            text.contains(&format!("http-equiv=\"refresh\" content=\"0; url={URL}\"")),
            "{text}"
        );
        assert!(!text.contains("<script"));
        drop(page);
        assert!(!path.exists());
    }

    /// A page the browser could not be handed is removed, and a stale page
    /// is replaced.
    #[test]
    fn a_failed_open_removes_the_page_and_a_stale_one_is_replaced() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("view-4173.html");
        fs::write(&path, "stale").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        let failed = LaunchPage::open(
            dir.path(),
            LocalPort::try_new(PORT).unwrap(),
            URL,
            |opened| {
                assert_eq!(
                    fs::metadata(opened).unwrap().permissions().mode() & 0o777,
                    0o600
                );
                Err(OpenError::Io(io::Error::other("no browser")))
            },
        );

        assert!(matches!(failed, Err(WebError::Browser(_))));
        assert!(!path.exists());
    }
}

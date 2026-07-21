use std::{
    fs,
    path::{MAIN_SEPARATOR, Path, PathBuf, is_separator},
};

use directories::BaseDirs;

use crate::domain::port::PathCompleter;

/// Leading path component expanded to the user's home directory.
const HOME_PREFIX: &str = "~";
/// Maximum directory suggestions offered at once.
const MAX_COMPLETIONS: usize = 8;

/// A [`PathCompleter`] that lists directories on the local filesystem.
#[derive(Default)]
pub struct FsPathCompleter;

impl PathCompleter for FsPathCompleter {
    fn complete_dir(&self, partial: &str) -> Vec<String> {
        complete_dir(partial)
    }
}

/// Subdirectories of the directory `partial` names (or its parent), matched by
/// the last path segment and re-joined onto whatever prefix the user typed.
/// Hidden entries appear only when the segment itself starts with a dot.
fn complete_dir(partial: &str) -> Vec<String> {
    // When `partial` is itself an existing directory (and not already ending in
    // a separator), browse inside it; otherwise complete the last segment within
    // its parent. This makes a prefilled path with no trailing separator list its
    // children rather than its siblings. `is_separator` accepts both `/` and the
    // platform separator, so Windows-style paths split correctly too.
    let (typed_dir, prefix): (String, String) =
        if !partial.ends_with(is_separator) && expand_home(Path::new(partial)).is_dir() {
            (format!("{partial}{MAIN_SEPARATOR}"), String::new())
        } else {
            match partial.rfind(is_separator) {
                Some(index) => (
                    partial[..=index].to_string(),
                    partial[index + 1..].to_string(),
                ),
                None => (String::new(), partial.to_string()),
            }
        };
    let read_dir = if typed_dir.is_empty() {
        PathBuf::from(".")
    } else {
        expand_home(Path::new(&typed_dir))
    };
    let Ok(entries) = fs::read_dir(&read_dir) else {
        return Vec::new();
    };
    let mut matches: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let raw = entry.file_name();
            // A lossy conversion would name a different path than was read, so a
            // non-UTF-8 entry cannot be a valid completion for this string port.
            let name = raw.to_str()?;
            let hidden = name.starts_with('.') && !prefix.starts_with('.');
            (name.starts_with(prefix.as_str()) && name != prefix && !hidden)
                .then(|| format!("{typed_dir}{name}"))
        })
        .collect();
    matches.sort();
    matches.truncate(MAX_COMPLETIONS);
    matches
}

/// Expands a leading `~` to the user's home directory; any other path is left
/// unchanged.
pub fn expand_home(path: &Path) -> PathBuf {
    match path.strip_prefix(HOME_PREFIX) {
        Ok(tail) => match BaseDirs::new() {
            Some(dirs) => dirs.home_dir().join(tail),
            None => path.to_path_buf(),
        },
        Err(_) => path.to_path_buf(),
    }
}

/// Normalizes a config path for identity comparison: expands `~`, resolves it
/// against the current directory when relative, and canonicalizes it when the
/// file exists (falling back to the absolute lexical path when it does not). Two
/// paths that name the same file, whether relative, absolute, or `~`-prefixed,
/// normalize to the same value.
pub fn normalize(path: &Path) -> PathBuf {
    let expanded = expand_home(path);
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(&expanded),
            Err(_) => expanded,
        }
    };
    absolute.canonicalize().unwrap_or(absolute)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_home_rewrites_a_tilde_prefix() {
        let home = BaseDirs::new().unwrap().home_dir().to_path_buf();
        assert_eq!(
            expand_home(Path::new("~/Projects/muster.yml")),
            home.join("Projects/muster.yml")
        );
        assert_eq!(
            expand_home(Path::new("/etc/muster.yml")),
            PathBuf::from("/etc/muster.yml")
        );
    }

    #[test]
    fn relative_and_absolute_paths_to_one_file_normalize_equal() {
        // `cargo test` runs with the crate root as the working directory, so the
        // relative name and its absolute form resolve to the same file.
        let absolute = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        assert_eq!(normalize(Path::new("Cargo.toml")), normalize(&absolute));
    }

    #[test]
    fn complete_dir_suggests_matching_subdirectories() {
        let dir = std::env::temp_dir().join(format!("muster-complete-{}", std::process::id()));
        for sub in ["prism", "proto", "other"] {
            fs::create_dir_all(dir.join(sub)).unwrap();
        }
        fs::write(dir.join("prfile"), "").unwrap();

        let base = dir.display();
        let got = complete_dir(&format!("{base}/pr"));

        assert_eq!(
            got,
            vec![format!("{base}/prism"), format!("{base}/proto")],
            "only matching directories, prefixed as typed, sorted"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn complete_dir_browses_inside_an_existing_directory() {
        let dir = std::env::temp_dir().join(format!("muster-browse-{}", std::process::id()));
        for sub in ["alpha", "beta"] {
            fs::create_dir_all(dir.join(sub)).unwrap();
        }

        // The path is an existing directory with no trailing slash: list its
        // children, not its siblings.
        let base = dir.display();
        let got = complete_dir(&base.to_string());

        assert_eq!(got, vec![format!("{base}/alpha"), format!("{base}/beta")]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn complete_dir_skips_non_utf8_entries() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

        let dir = std::env::temp_dir().join(format!("muster-nonutf8-{}", std::process::id()));
        fs::create_dir_all(dir.join("valid")).unwrap();
        // A directory whose name is not valid UTF-8 but shares the "val" prefix.
        fs::create_dir_all(dir.join(OsStr::from_bytes(b"val\xffid"))).unwrap();

        let base = dir.display();
        let got = complete_dir(&format!("{base}/val"));

        assert_eq!(
            got,
            vec![format!("{base}/valid")],
            "the non-UTF-8 entry is skipped, not offered lossily"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}

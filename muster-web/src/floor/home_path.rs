use std::path::Path;

/// What stands for the home folder in a path the page shows.
const HOME_MARK: &str = "~";

/// `path` as the page shows it: written from `~` when it is under `home`. A
/// home of `/` is ignored, so not every path turns into one under `~`.
pub(super) fn home_relative(path: &Path, home: Option<&Path>) -> String {
    let rest = home
        .filter(|home| home.parent().is_some())
        .and_then(|home| path.strip_prefix(home).ok());
    match rest {
        Some(rest) if rest.as_os_str().is_empty() => HOME_MARK.to_string(),
        Some(rest) => Path::new(HOME_MARK).join(rest).display().to_string(),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path under home shows from `~`; others show whole.
    #[test]
    fn the_home_folder_shows_as_a_tilde() {
        let home = Some(Path::new("/home/me"));

        assert_eq!(home_relative(Path::new("/home/me/shop"), home), "~/shop");
        assert_eq!(home_relative(Path::new("/home/me"), home), "~");
        assert_eq!(
            home_relative(Path::new("/home/meadow/shop"), home),
            "/home/meadow/shop"
        );
        assert_eq!(home_relative(Path::new("/srv/shop"), home), "/srv/shop");
        assert_eq!(home_relative(Path::new("/srv/shop"), None), "/srv/shop");
        assert_eq!(
            home_relative(Path::new("/srv/shop"), Some(Path::new("/"))),
            "/srv/shop"
        );
    }
}

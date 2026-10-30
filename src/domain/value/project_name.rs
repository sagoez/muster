use std::iter;

use nutype::nutype;

use super::{ProjectRoot, printable, steers_the_terminal};

/// The name a project takes when its folder has none: the filesystem root.
const ROOT_FOLDER_NAME: &str = "root";
/// The first suffix tried when a project's folder name is already taken.
const FIRST_COLLISION_SUFFIX: u64 = 2;

/// Whether `name` holds nothing that could steer a terminal it is shown on.
fn is_terminal_safe(name: &str) -> bool {
    !name.chars().any(steers_the_terminal)
}

/// A registered project's display name: trimmed, non-empty and free of
/// control and format characters.
#[nutype(
    sanitize(trim),
    validate(not_empty, predicate = is_terminal_safe),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        AsRef,
        Display,
        Serialize,
        Deserialize
    )
)]
pub struct ProjectName(String);

impl ProjectName {
    /// The folder name of `root` with anything terminal-steering escaped,
    /// then `<name>-2`, `<name>-3`, ... for collisions.
    pub fn candidates(root: &ProjectRoot) -> impl Iterator<Item = ProjectName> {
        let base = root
            .as_ref()
            .file_name()
            .map(|name| printable(&name.to_string_lossy()))
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| ROOT_FOLDER_NAME.to_string());
        let suffixed = (FIRST_COLLISION_SUFFIX..).map({
            let base = base.clone();
            move |suffix| format!("{base}-{suffix}")
        });
        iter::once(base)
            .chain(suffixed)
            .filter_map(|raw| Self::try_new(raw).ok())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// The project root at `path`.
    fn root(path: &str) -> ProjectRoot {
        ProjectRoot::try_new(PathBuf::from(path)).unwrap()
    }

    #[test]
    fn trims_and_rejects_empty() {
        assert_eq!(
            ProjectName::try_new("  muster  ").unwrap().as_ref(),
            "muster"
        );
        assert!(ProjectName::try_new("   ").is_err());
    }

    /// The folder name comes first, then suffixed variants from `-2`.
    #[test]
    fn candidates_start_at_the_folder_name_then_suffix() {
        let names: Vec<String> = ProjectName::candidates(&root("/home/me/shop"))
            .take(3)
            .map(|name| name.to_string())
            .collect();

        assert_eq!(names, ["shop", "shop-2", "shop-3"]);
    }

    /// A name carrying an OSC title sequence or a bidi override is refused.
    #[test]
    fn control_and_format_characters_are_refused() {
        assert!(ProjectName::try_new("shop\u{1b}]0;pwned\u{7}").is_err());
        assert!(ProjectName::try_new("shop\u{202e}").is_err());
        assert!(ProjectName::try_new("caf\u{e9} shop").is_ok());
    }

    /// A folder whose name carries OSC sequences yields an escaped, valid name.
    #[test]
    fn a_hostile_folder_name_is_escaped() {
        let first = ProjectName::candidates(&root("/home/me/x\u{1b}]0;pwned\u{7}"))
            .next()
            .unwrap();

        assert_eq!(first.as_ref(), "x\\u{1b}]0;pwned\\u{7}");
    }

    #[test]
    fn the_filesystem_root_has_a_name() {
        let first = ProjectName::candidates(&root("/")).next().unwrap();

        assert_eq!(first.as_ref(), ROOT_FOLDER_NAME);
    }
}

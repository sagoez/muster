use std::iter;

use nutype::nutype;

use super::ProjectRoot;

/// The name a project takes when its folder has none: the filesystem root.
const ROOT_FOLDER_NAME: &str = "root";
/// The first suffix tried when a project's folder name is already taken.
const FIRST_COLLISION_SUFFIX: u64 = 2;

/// A registered project's display name: trimmed and non-empty.
#[nutype(
    sanitize(trim),
    validate(not_empty),
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
    /// The folder name of `root`, then `<name>-2`, `<name>-3`, ... for collisions.
    pub fn candidates(root: &ProjectRoot) -> impl Iterator<Item = ProjectName> {
        let base = root
            .as_ref()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
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

    #[test]
    fn the_filesystem_root_has_a_name() {
        let first = ProjectName::candidates(&root("/")).next().unwrap();

        assert_eq!(first.as_ref(), ROOT_FOLDER_NAME);
    }
}

use std::path::{Path, PathBuf};

use nutype::nutype;

/// A project's folder: absolute; canonicalized by the adapter that builds it.
#[nutype(
    validate(predicate = |root: &PathBuf| root.is_absolute()),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        PartialOrd,
        Ord,
        AsRef,
        Serialize,
        Deserialize
    )
)]
pub struct ProjectRoot(PathBuf);

impl ProjectRoot {
    /// Whether `dir` is this folder or inside it, compared by components.
    pub fn contains(&self, dir: &Path) -> bool {
        dir.starts_with(self.as_ref())
    }

    /// `dir` relative to this root when inside it.
    pub fn relative(&self, dir: &Path) -> Option<PathBuf> {
        dir.strip_prefix(self.as_ref()).ok().map(Path::to_path_buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The project root at `path`.
    fn root(path: &str) -> ProjectRoot {
        ProjectRoot::try_new(PathBuf::from(path)).unwrap()
    }

    #[test]
    fn refuses_a_relative_folder() {
        assert!(ProjectRoot::try_new(PathBuf::from("shop")).is_err());
    }

    /// Containment is by whole components, so a sibling sharing a prefix is outside.
    #[test]
    fn contains_itself_and_descendants_only() {
        let shop = root("/home/me/shop");

        assert!(shop.contains(Path::new("/home/me/shop")));
        assert!(shop.contains(Path::new("/home/me/shop/web/src")));
        assert!(!shop.contains(Path::new("/home/me/shopping")));
        assert!(!shop.contains(Path::new("/home/me")));
    }

    #[test]
    fn relative_strips_the_root() {
        let shop = root("/home/me/shop");

        assert_eq!(
            shop.relative(Path::new("/home/me/shop/web")),
            Some(PathBuf::from("web"))
        );
        assert_eq!(
            shop.relative(Path::new("/home/me/shop")),
            Some(PathBuf::new())
        );
        assert_eq!(shop.relative(Path::new("/home/me/shopping")), None);
    }
}

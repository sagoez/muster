use std::path::{Component, Path};

/// Whether `dir`, a working directory as stored, names a folder under the
/// project root: relative to it, and never climbing above it as written.
/// Symlinks are not followed; writers check those against the disk.
pub fn stays_under_root(dir: &Path) -> bool {
    let mut depth: usize = 0;
    for component in dir.components() {
        depth = match component {
            Component::Normal(_) => depth + 1,
            Component::CurDir => depth,
            Component::ParentDir => match depth.checked_sub(1) {
                Some(depth) => depth,
                None => return false,
            },
            Component::RootDir | Component::Prefix(_) => return false,
        };
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A relative folder that may step up but never above the root stays
    /// under it; an absolute one or one climbing out does not.
    #[test]
    fn only_a_relative_dir_that_never_climbs_out_stays_under_the_root() {
        for inside in ["frontend", "./a/b", "a/../b", "a/b/.."] {
            assert!(stays_under_root(Path::new(inside)), "{inside}");
        }
        for outside in ["/etc", "..", "../elsewhere", "a/../../b", "../../../../etc"] {
            assert!(!stays_under_root(Path::new(outside)), "{outside}");
        }
    }
}

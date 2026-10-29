use std::path::PathBuf;

use nutype::nutype;

use crate::adapter::path;

/// Identity of one project: its config path, made absolute with every symlink
/// resolved, so each alias of one `muster.yml` names one project. A config that
/// cannot be resolved, as one not yet written, is only made absolute.
#[nutype(
    sanitize(with = |config: PathBuf| path::normalize(&config)),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Serialize, Deserialize),
)]
pub struct ProjectKey(PathBuf);

#[cfg(test)]
mod tests {
    use std::{env, fs, os::unix::fs::symlink};

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn a_relative_config_path_is_anchored_to_the_working_directory() {
        let key = ProjectKey::new(PathBuf::from("not-yet-written/muster.yml"));

        assert_eq!(
            key.as_ref(),
            &env::current_dir()
                .unwrap()
                .join("not-yet-written/muster.yml")
        );
    }

    /// A config reached through a linked folder or a linked file resolves to
    /// the same project as the config itself.
    #[test]
    fn symlink_aliases_of_one_config_resolve_to_one_project() {
        let scratch = TempDir::new().unwrap();
        let real = scratch.path().join("real");
        fs::create_dir(&real).unwrap();
        let config = real.join("muster.yml");
        fs::write(&config, b"agents: []").unwrap();
        symlink(&real, scratch.path().join("linked-dir")).unwrap();
        symlink(&config, scratch.path().join("linked.yml")).unwrap();

        let key = ProjectKey::new(config.clone());

        assert_eq!(
            ProjectKey::new(scratch.path().join("linked-dir/muster.yml")),
            key
        );
        assert_eq!(ProjectKey::new(scratch.path().join("linked.yml")), key);
    }
}

use std::path::{Path, PathBuf};

use getset::Getters;
use muster_domain::{
    coordination::{LOCK_NAME_MAX_CHARS, Lock, LockName, Todo},
    project::Project,
};
use muster_protocol::path_key;
use typed_builder::TypedBuilder;

/// Opens every checkout lock's name.
const LOCK_PREFIX: &str = "checkout-";
/// Joins the folder's name to its key, and stands in for every character a
/// lock name may not hold.
const FILL: char = '-';

/// A git working tree a call changes or publishes, with what decides whether
/// it may: its lock, the project that lock lives in, and that project's live
/// locks and open todos.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct CheckoutScope {
    /// The working tree's root, as `git rev-parse --show-toplevel` names it.
    root: PathBuf,
    /// Its lock, as [`checkout_lock`] names it.
    lock: LockName,
    /// The project its lock lives in: the one containing the root, else the
    /// session's.
    project: Project,
    /// That project's live locks.
    locks: Vec<Lock>,
    /// That project's open todos, where the lock's holder announced its work.
    todos: Vec<Todo>,
}

/// The lock of the working tree at `root`: `checkout-<folder>-<key of the
/// root>`, as readable as a lock name allows and unique to the root.
pub fn checkout_lock(root: &Path) -> Option<LockName> {
    let key = path_key(root);
    let room = LOCK_NAME_MAX_CHARS - LOCK_PREFIX.len() - key.len() - 1;
    let folder: String = root
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
        .chars()
        .map(|character| {
            if character.is_ascii_lowercase() || character.is_ascii_digit() {
                character
            } else {
                FILL
            }
        })
        .take(room)
        .collect();
    let folder = folder.trim_matches(FILL);
    let name = if folder.is_empty() {
        format!("{LOCK_PREFIX}{key}")
    } else {
        format!("{LOCK_PREFIX}{folder}{FILL}{key}")
    };
    LockName::try_new(name).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lock names the folder readably, tells two roots of one name apart,
    /// and stays a valid name for a long, odd or bare root.
    #[test]
    fn a_checkout_lock_is_readable_unique_and_valid() {
        let shop = checkout_lock(Path::new("/work/Shop App")).unwrap();
        assert!(shop.as_ref().starts_with("checkout-shop-app-"), "{shop}");
        let other = checkout_lock(Path::new("/elsewhere/Shop App")).unwrap();
        assert_ne!(shop, other);
        let long = format!("/work/{}", "x".repeat(LOCK_NAME_MAX_CHARS * 2));
        for root in [long.as_str(), "/", "/work/---"] {
            assert!(checkout_lock(Path::new(root)).is_some(), "{root}");
        }
    }
}

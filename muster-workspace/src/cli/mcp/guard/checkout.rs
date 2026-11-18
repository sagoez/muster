use std::path::{Path, PathBuf};

use getset::Getters;
use muster_domain::coordination::{LOCK_NAME_MAX_CHARS, LockName};
use muster_protocol::path_key;
use typed_builder::TypedBuilder;

/// Opens every checkout lock's name.
const LOCK_PREFIX: &str = "checkout-";
/// Joins the folder's name to its key, and stands in for every character a
/// lock name may not hold.
const FILL: char = '-';

/// One git working tree, by its root, and the lock an agent holds while it
/// changes or publishes it.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Checkout {
    /// The working tree's root, as `git rev-parse --show-toplevel` names it.
    root: PathBuf,
    /// Its lock: `checkout-<folder name>-<key of the root>`, as readable as
    /// a lock name allows and unique to the root.
    lock: LockName,
}

impl Checkout {
    /// The checkout rooted at `root`; none only if its lock name would not
    /// validate, which its construction rules out.
    pub fn at(root: &Path) -> Option<Self> {
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
        Some(Self {
            root: root.to_path_buf(),
            lock: LockName::try_new(name).ok()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lock names the folder readably, tells two roots of one name apart,
    /// and stays a valid name for a long, odd or bare root.
    #[test]
    fn a_checkout_lock_is_readable_unique_and_valid() {
        let shop = Checkout::at(Path::new("/work/Shop App")).unwrap();
        assert!(
            shop.lock().as_ref().starts_with("checkout-shop-app-"),
            "{shop:?}"
        );
        let other = Checkout::at(Path::new("/elsewhere/Shop App")).unwrap();
        assert_ne!(shop.lock(), other.lock());
        let long = format!("/work/{}", "x".repeat(LOCK_NAME_MAX_CHARS * 2));
        assert!(Checkout::at(Path::new(&long)).is_some());
        assert!(Checkout::at(Path::new("/")).is_some());
        assert!(Checkout::at(Path::new("/work/---")).is_some());
    }
}

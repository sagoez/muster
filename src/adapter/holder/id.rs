use nutype::nutype;

/// Unique identity of one holder, safe to use as a file name: non-empty ASCII
/// alphanumerics, hyphens and underscores only.
#[nutype(
    validate(predicate = |id: &str| !id.is_empty()
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')),
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
pub struct HolderId(String);

impl HolderId {
    /// Creates a new globally unique holder identity.
    ///
    /// # Errors
    /// Returns the newtype's validation error if a UUID's simple form ever
    /// stops satisfying the file-name rule.
    pub fn generate() -> Result<Self, HolderIdError> {
        Self::try_new(uuid::Uuid::new_v4().simple().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_path_separators_and_empty_ids() {
        assert!(HolderId::try_new("../escape").is_err());
        assert!(HolderId::try_new("").is_err());
        assert!(HolderId::generate().is_ok());
    }
}

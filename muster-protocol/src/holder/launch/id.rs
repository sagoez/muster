use std::path::Path;

use nutype::nutype;
use uuid::Uuid;

/// Hex digits of a generated holder id: 64 random bits, plenty for one
/// user's holders, and short enough that a holder's socket path fits in a
/// Unix socket address under a long home folder.
const HOLDER_ID_DIGITS: usize = 16;

/// Unique identity of one holder, safe to use as a file name: non-empty ASCII
/// alphanumerics, hyphens and underscores only. Any such length is accepted,
/// so a running holder an earlier muster named with a longer id is still
/// adopted.
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
    /// Creates a new random holder identity of [`HOLDER_ID_DIGITS`] hex
    /// digits. The two halves of a v4 UUID are folded together, so the bits
    /// each half fixes for the version and variant meet random ones.
    ///
    /// # Errors
    /// Returns the newtype's validation error if hex digits ever stop
    /// satisfying the file-name rule.
    pub fn generate() -> Result<Self, HolderIdError> {
        let (high, low) = Uuid::new_v4().as_u64_pair();
        Self::try_new(format!("{:0width$x}", high ^ low, width = HOLDER_ID_DIGITS))
    }

    /// The holder id a socket, handover or record file's name carries as
    /// its stem, when it is a valid one.
    pub fn of_file(path: &Path) -> Option<Self> {
        Self::try_new(path.file_stem()?.to_str()?).ok()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    /// How many ids the uniqueness check draws.
    const DRAWS: usize = 1000;
    /// An id as an earlier muster generated it: a simple UUID.
    const EARLIER_ID: &str = "0f8e3a1c9b7d4e2fa6c5b4d3e2f1a0b9";

    #[test]
    fn refuses_path_separators_and_empty_ids() {
        assert!(HolderId::try_new("../escape").is_err());
        assert!(HolderId::try_new("").is_err());
        assert!(HolderId::generate().is_ok());
    }

    /// A generated id is 16 hex digits, so a holder socket path stays short,
    /// and ids do not repeat.
    #[test]
    fn a_generated_id_is_sixteen_random_hex_digits() {
        let ids: HashSet<HolderId> = (0..DRAWS).map(|_| HolderId::generate().unwrap()).collect();

        assert_eq!(ids.len(), DRAWS);
        for id in &ids {
            assert_eq!(id.as_ref().len(), HOLDER_ID_DIGITS, "{id}");
            assert!(
                id.as_ref().bytes().all(|byte| byte.is_ascii_hexdigit()),
                "{id}"
            );
        }
    }

    /// An id an earlier muster wrote in a running holder's record is still
    /// valid, so that holder is adopted.
    #[test]
    fn an_earlier_longer_id_is_still_valid() {
        assert_eq!(HolderId::try_new(EARLIER_ID).unwrap().as_ref(), EARLIER_ID);
    }

    /// A file named for a holder, whatever its extension, carries its id;
    /// a name that is no id carries none.
    #[test]
    fn a_holder_files_stem_is_its_id() {
        let id = HolderId::try_new(EARLIER_ID).unwrap();

        for extension in ["sock", "json", "err"] {
            let file = Path::new("/tmp").join(EARLIER_ID).with_extension(extension);
            assert_eq!(HolderId::of_file(&file), Some(id.clone()));
        }
        assert_eq!(HolderId::of_file(Path::new("/tmp/not an id.sock")), None);
    }
}

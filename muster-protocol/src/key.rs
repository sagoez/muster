//! Short stable names for long values: the service socket, the holder
//! sockets' fallback folder and the process log folders are named by them.

use std::path::Path;

/// FNV-1a's 64-bit offset basis.
const FNV_OFFSET_BASIS: u64 = 0xCBF2_9CE4_8422_2325;
/// FNV-1a's 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;
/// Hex digits of a key: every bit of the 64-bit hash.
pub(crate) const KEY_DIGITS: usize = 16;

/// The 64-bit FNV-1a hash of `bytes`: stable across builds and platforms,
/// unlike the standard library's hasher.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

/// The [`fnv1a`] hash of `bytes` as [`KEY_DIGITS`] hex digits.
pub(crate) fn hash_key(bytes: &[u8]) -> String {
    format!("{:0width$x}", fnv1a(bytes), width = KEY_DIGITS)
}

/// The key of `path`, so a long path names a short file or folder.
pub(crate) fn path_key(path: &Path) -> String {
    hash_key(path.as_os_str().as_encoded_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hash is the published FNV-1a, so the names it makes survive
    /// rebuilds, and a key is always its full width.
    #[test]
    fn the_key_is_stable_zero_padded_fnv1a() {
        assert_eq!(fnv1a(b""), FNV_OFFSET_BASIS);
        assert_eq!(fnv1a(b"a"), 0xAF63_DC4C_8601_EC8C);
        assert_eq!(hash_key(b"a"), "af63dc4c8601ec8c");
        assert_eq!(path_key(Path::new("a")), hash_key(b"a"));
    }
}

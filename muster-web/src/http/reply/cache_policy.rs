use strum::EnumProperty;

/// The property holding a policy's `Cache-Control` value.
const HEADER_VALUE: &str = "value";

/// How long a browser may keep a response.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumProperty)]
pub enum CachePolicy {
    /// Never kept: the shell, the API and every refusal.
    #[strum(props(value = "no-store"))]
    NoStore,
    /// Kept for a year: an asset, whose path carries the version.
    #[strum(props(value = "public, max-age=31536000, immutable"))]
    Immutable,
}

impl CachePolicy {
    /// The `Cache-Control` value.
    pub fn header_value(self) -> &'static str {
        self.get_str(HEADER_VALUE).unwrap_or_default()
    }
}

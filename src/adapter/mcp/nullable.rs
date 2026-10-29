use serde::{Deserialize, Deserializer};

/// Reads a field that may be absent, `null` or a value, so an argument can
/// tell "keep" (absent, through `#[serde(default)]`) from "clear" (`null`).
///
/// # Errors
/// Returns the deserializer's error when the value is not a `T`.
pub(super) fn nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

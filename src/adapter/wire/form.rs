//! Serde `with` encoding of one domain value through its [`WireForm`].

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

use super::WireForm;

/// Writes `value` as its DTO.
///
/// # Errors
/// Returns the serializer's error.
pub fn serialize<T: WireForm, S: Serializer>(value: &T, serializer: S) -> Result<S::Ok, S::Error> {
    value.to_wire().serialize(serializer)
}

/// Reads a DTO and the value it carries.
///
/// # Errors
/// Returns the deserializer's error, or why the DTO holds no valid value.
pub fn deserialize<'de, T: WireForm, D: Deserializer<'de>>(deserializer: D) -> Result<T, D::Error> {
    T::from_wire(T::Wire::deserialize(deserializer)?).map_err(D::Error::custom)
}

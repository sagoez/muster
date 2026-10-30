//! Serde `with` encoding of an optional domain value through its [`WireForm`].

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

use super::WireForm;

/// Writes `value` as its DTO, or as none.
///
/// # Errors
/// Returns the serializer's error.
pub fn serialize<T: WireForm, S: Serializer>(
    value: &Option<T>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value.as_ref().map(WireForm::to_wire).serialize(serializer)
}

/// Reads an optional DTO and the value it carries.
///
/// # Errors
/// Returns the deserializer's error, or why the DTO holds no valid value.
pub fn deserialize<'de, T: WireForm, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T::Wire>::deserialize(deserializer)?
        .map(T::from_wire)
        .transpose()
        .map_err(D::Error::custom)
}

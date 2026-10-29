//! Serde `with` encoding of a list of domain values through their [`WireForm`].

use serde::{Deserialize, Deserializer, Serializer, de::Error};

use super::WireForm;

/// Writes `values` as a sequence of their DTOs.
///
/// # Errors
/// Returns the serializer's error.
pub fn serialize<T: WireForm, S: Serializer>(
    values: &[T],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(values.iter().map(WireForm::to_wire))
}

/// Reads a sequence of DTOs and the values they carry.
///
/// # Errors
/// Returns the deserializer's error, or why a DTO holds no valid value.
pub fn deserialize<'de, T: WireForm, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<T>, D::Error> {
    Vec::<T::Wire>::deserialize(deserializer)?
        .into_iter()
        .map(T::from_wire)
        .collect::<Result<_, _>>()
        .map_err(D::Error::custom)
}

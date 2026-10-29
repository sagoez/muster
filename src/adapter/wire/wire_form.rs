use std::{fmt::Display, str::FromStr};

use serde::{Serialize, de::DeserializeOwned};

use super::TextForm;

/// How a domain value travels on a wire or sits in a file. The domain derives
/// no serde; each adapter DTO is named here as the value's `Wire`, so a field
/// keeps its domain type and states its encoding with the `form` modules.
pub trait WireForm: Sized {
    /// The adapter DTO the value travels as.
    type Wire: Serialize + DeserializeOwned;

    /// This value as its DTO.
    fn to_wire(&self) -> Self::Wire;

    /// The value a DTO carries.
    ///
    /// # Errors
    /// Returns why the DTO holds no valid value.
    fn from_wire(wire: Self::Wire) -> Result<Self, String>;
}

impl<T> WireForm for T
where
    T: TextForm + Display + FromStr,
    T::Err: Display,
{
    type Wire = String;

    /// The value's text, as `Display` writes it.
    fn to_wire(&self) -> String {
        self.to_string()
    }

    /// The value its text names, as `FromStr` reads it.
    fn from_wire(wire: String) -> Result<Self, String> {
        wire.parse().map_err(|error: T::Err| error.to_string())
    }
}

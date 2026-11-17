use std::fmt::{self, Display, Formatter};

use uuid::Uuid;

/// The fence value a test's block carries, so its output is the same every
/// run.
#[cfg(test)]
const FIXED: &str = "f3nce";

/// The value fencing one run's shared block: random and fresh for each
/// run, so no stored text can name the tag that closes it. Hand-written, not
/// a nutype, so nothing outside can build one from a fixed or empty string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fence(String);

impl Fence {
    /// A fence made fresh from a random v4 UUID.
    pub fn fresh() -> Self {
        Self(Uuid::new_v4().simple().to_string())
    }

    /// The fence [`FIXED`], for a test's output.
    #[cfg(test)]
    pub fn fixed() -> Self {
        Self(FIXED.to_owned())
    }
}

impl Display for Fence {
    /// Writes the fence value.
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every fresh fence is a full random id, never one seen before.
    #[test]
    fn each_fence_is_fresh() {
        let (one, two) = (Fence::fresh(), Fence::fresh());
        assert_ne!(one, two);
        assert_eq!(one.to_string().len(), 32, "{one}");
    }
}

use serde::{Deserialize, Serialize};

use super::form;
use crate::domain::process::Readiness;

/// A record carrying a text-form value, for the text-form tests.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct TextCarrier {
    #[serde(with = "form")]
    ready: Readiness,
}

impl TextCarrier {
    /// A record carrying `ready`.
    pub(super) fn new(ready: Readiness) -> Self {
        Self { ready }
    }
}

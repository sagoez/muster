use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::{WireForm, form};
use crate::domain::process::{LogPattern, PatternKind, PatternText};

/// A pattern one output line is tested against, as a wait carries it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct LogPatternWire {
    /// What to look for.
    text: PatternText,
    /// How to look for it.
    #[serde(with = "form")]
    kind: PatternKind,
}

impl WireForm for LogPattern {
    type Wire = LogPatternWire;

    fn to_wire(&self) -> LogPatternWire {
        LogPatternWire {
            text: self.text().clone(),
            kind: *self.kind(),
        }
    }

    fn from_wire(wire: LogPatternWire) -> Result<Self, String> {
        Ok(Self::builder().text(wire.text).kind(wire.kind).build())
    }
}

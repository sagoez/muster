use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::WireForm;
use crate::domain::{
    pty::PtySize,
    value::{Cols, Rows},
};

/// A PTY size, as a holder's spawn request carries it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct PtySizeWire {
    /// Rows, within the PTY bounds.
    rows: Rows,
    /// Columns, within the PTY bounds.
    cols: Cols,
}

impl WireForm for PtySize {
    type Wire = PtySizeWire;

    fn to_wire(&self) -> PtySizeWire {
        PtySizeWire {
            rows: self.rows(),
            cols: self.cols(),
        }
    }

    fn from_wire(wire: PtySizeWire) -> Result<Self, String> {
        Ok(Self::builder().rows(wire.rows).cols(wire.cols).build())
    }
}

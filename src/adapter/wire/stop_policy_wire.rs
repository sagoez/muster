use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::{WireForm, form};
use crate::domain::{
    process::{StopPolicy, StopSignal},
    value::StopGrace,
};

/// How a process stops, as a holder keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(deny_unknown_fields)]
pub struct StopPolicyWire {
    /// The graceful signal.
    #[serde(with = "form")]
    signal: StopSignal,
    /// Time allowed before a force-kill.
    grace_period: StopGrace,
}

impl WireForm for StopPolicy {
    type Wire = StopPolicyWire;

    fn to_wire(&self) -> StopPolicyWire {
        StopPolicyWire {
            signal: *self.signal(),
            grace_period: *self.grace_period(),
        }
    }

    fn from_wire(wire: StopPolicyWire) -> Result<Self, String> {
        Ok(Self::builder()
            .signal(wire.signal)
            .grace_period(wire.grace_period)
            .build())
    }
}

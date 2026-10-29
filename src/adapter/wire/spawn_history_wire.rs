use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::{WireForm, optional_form};
use crate::domain::pty::{ExitOutcome, SpawnHistory};

/// What a process's earlier runs leave to the next, as a holder keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct SpawnHistoryWire {
    /// Times the process was restarted.
    restarts: u32,
    /// How the previous run ended, when there was one.
    #[serde(with = "optional_form")]
    last_exit: Option<ExitOutcome>,
}

impl WireForm for SpawnHistory {
    type Wire = SpawnHistoryWire;

    fn to_wire(&self) -> SpawnHistoryWire {
        SpawnHistoryWire {
            restarts: self.restarts(),
            last_exit: self.last_exit().clone(),
        }
    }

    fn from_wire(wire: SpawnHistoryWire) -> Result<Self, String> {
        Ok(Self::builder()
            .restarts(wire.restarts)
            .last_exit(wire.last_exit)
            .build())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pty::ExitCause;

    /// A history travels in its kebab-case form and comes back whole.
    #[test]
    fn a_history_round_trips_in_its_kebab_case_form() {
        let history = SpawnHistory::builder()
            .restarts(2)
            .last_exit(Some(ExitOutcome::Failed(ExitCause::Status(3))))
            .build();

        let json = serde_json::to_string(&history.to_wire()).unwrap();

        assert_eq!(
            json,
            r#"{"restarts":2,"last-exit":{"failed":{"status":3}}}"#
        );
        let wire: SpawnHistoryWire = serde_json::from_str(&json).unwrap();
        assert_eq!(SpawnHistory::from_wire(wire).unwrap(), history);
    }

    /// A field this muster does not know is refused rather than dropped.
    #[test]
    fn an_unknown_field_is_refused() {
        let refused =
            serde_json::from_str::<SpawnHistoryWire>(r#"{"restarts":0,"last-exit":null,"x":1}"#);

        assert!(refused.is_err());
    }
}

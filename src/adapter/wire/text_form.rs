use crate::domain::{
    color_scheme::ColorScheme,
    coordination::{Author, TodoStatus},
    process::{PatternKind, ProcessState, Readiness, RestartPolicy, StopSignal},
};

/// A domain value that travels as the text its `Display` writes and its
/// `FromStr` reads back, so it takes its [`super::WireForm`] from that text.
pub trait TextForm {}

impl TextForm for Author {}
impl TextForm for ColorScheme {}
impl TextForm for PatternKind {}
impl TextForm for ProcessState {}
impl TextForm for Readiness {}
impl TextForm for RestartPolicy {}
impl TextForm for StopSignal {}
impl TextForm for TodoStatus {}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::adapter::wire::form;

    /// A record carrying a text-form value.
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Carrier {
        #[serde(with = "form")]
        ready: Readiness,
    }

    /// A text-form value travels as its kebab-case name and refuses any
    /// other text.
    #[test]
    fn a_text_form_travels_as_its_name_and_refuses_others() {
        let carrier = Carrier {
            ready: Readiness::TimedOut,
        };

        let json = serde_json::to_string(&carrier).unwrap();

        assert_eq!(json, r#"{"ready":"timed-out"}"#);
        assert_eq!(serde_json::from_str::<Carrier>(&json).unwrap(), carrier);
        assert!(serde_json::from_str::<Carrier>(r#"{"ready":"late"}"#).is_err());
    }
}

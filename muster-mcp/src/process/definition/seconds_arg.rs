use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, de::Error as DeError};
use serde_json::{Number, Value};

/// What a duration argument must be, named when another shape is sent.
const SECONDS_FORM: &str = "a number of seconds such as 30";

/// A duration an agent writes as a number of seconds, the shape every result
/// reports one in. Text such as `"30s"` is refused.
#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
#[schemars(transparent)]
pub struct SecondsArg(#[schemars(with = "f64")] Number);

impl SecondsArg {
    /// The seconds as written, for a refusal to quote.
    pub fn written(&self) -> String {
        self.0.to_string()
    }

    /// The duration these seconds make: none when they are negative, not
    /// finite or too large to hold.
    pub fn duration(&self) -> Option<Duration> {
        self.0
            .as_f64()
            .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
    }
}

impl<'de> Deserialize<'de> for SecondsArg {
    /// Reads a JSON number, refusing any other value with an error that names
    /// seconds.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::Number(seconds) => Ok(Self(seconds)),
            other => Err(D::Error::custom(format!(
                "expected {SECONDS_FORM}, got {other}"
            ))),
        }
    }
}

#[cfg(test)]
impl From<u64> for SecondsArg {
    /// Whole seconds, as a test writes them.
    fn from(seconds: u64) -> Self {
        Self(seconds.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A number reads as its seconds, a fraction included.
    #[test]
    fn a_number_reads_as_seconds() {
        let read = |json| serde_json::from_value::<SecondsArg>(json).unwrap();

        assert_eq!(
            read(serde_json::json!(30)).duration(),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            read(serde_json::json!(1.5)).duration(),
            Some(Duration::from_millis(1500))
        );
        assert_eq!(read(serde_json::json!(-1)).duration(), None);
    }

    /// Text in humantime form is refused naming seconds.
    #[test]
    fn text_is_refused_naming_seconds() {
        let refused = serde_json::from_value::<SecondsArg>(serde_json::json!("30s"))
            .unwrap_err()
            .to_string();

        assert!(refused.contains("number of seconds"), "{refused}");
        assert!(refused.contains("\"30s\""), "{refused}");
    }

    /// The schema an agent reads says number.
    #[test]
    fn the_schema_is_a_number() {
        let schema = serde_json::to_value(schemars::schema_for!(SecondsArg)).unwrap();

        assert_eq!(schema["type"], "number", "{schema}");
    }
}

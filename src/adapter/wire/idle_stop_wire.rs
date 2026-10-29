use super::WireForm;
use crate::domain::value::{IdleStop, ValueParseError};

impl WireForm for IdleStop {
    type Wire = String;

    /// The idle stop's text, such as `30m` or `never`.
    fn to_wire(&self) -> String {
        self.to_string()
    }

    /// The idle stop its text names.
    fn from_wire(wire: String) -> Result<Self, String> {
        wire.parse()
            .map_err(|error: ValueParseError| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An idle stop travels as its text and refuses text that is not one.
    #[test]
    fn an_idle_stop_travels_as_its_text() {
        for raw in ["30m", "never"] {
            let idle_stop: IdleStop = raw.parse().unwrap();

            assert_eq!(idle_stop.to_wire(), raw);
            assert_eq!(IdleStop::from_wire(raw.to_string()), Ok(idle_stop));
        }
        assert!(IdleStop::from_wire("0s".to_string()).is_err());
    }
}

use std::str::FromStr;

use nutype::nutype;

use super::ValueParseError;

/// The form a port is written in.
const PORT_FORM: &str = "a port from 1 to 65535";

/// A TCP port on this machine that a readiness check or wait probes; zero
/// names no port.
#[nutype(
    validate(greater = 0),
    derive(
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
        PartialOrd,
        Ord,
        Hash,
        Display,
        Serialize,
        Deserialize
    )
)]
pub struct LocalPort(u16);

impl FromStr for LocalPort {
    type Err = ValueParseError;

    /// Reads a decimal port such as `3000`.
    ///
    /// # Errors
    /// Returns [`ValueParseError`] for text that is not a port from 1 to 65535.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        raw.trim()
            .parse()
            .ok()
            .and_then(|port| Self::try_new(port).ok())
            .ok_or_else(|| {
                ValueParseError::builder()
                    .expected(PORT_FORM)
                    .found(raw)
                    .build()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ports_and_refuses_zero_and_overflow() {
        assert_eq!(" 3000 ".parse::<LocalPort>().unwrap().into_inner(), 3000);
        assert_eq!("65535".parse::<LocalPort>().unwrap().into_inner(), 65535);
        for raw in ["0", "65536", "-1", "http", ""] {
            assert!(raw.parse::<LocalPort>().is_err(), "{raw}");
        }
        assert!(serde_json::from_str::<LocalPort>("0").is_err());
    }
}

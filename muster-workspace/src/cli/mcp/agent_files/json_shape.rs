use strum::Display;

/// The shape a part of a JSON file should have, as a refusal names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display)]
pub enum JsonShape {
    /// A JSON object.
    #[strum(to_string = "an object")]
    Object,
    /// A JSON array.
    #[strum(to_string = "a list")]
    List,
}

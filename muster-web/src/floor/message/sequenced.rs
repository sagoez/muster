use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::floor_message::FloorMessage;
use crate::WebError;

/// A message with the sequence number the floor gave it, which rises by one
/// per message so the page can drop a stale one.
#[derive(Clone, Debug, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub(crate) struct Sequenced {
    /// Its place in the floor's messages, from 1.
    #[getset(get_copy = "pub")]
    seq: u64,
    /// The message.
    #[getset(get = "pub")]
    #[serde(flatten)]
    message: FloorMessage,
}

impl Sequenced {
    /// The message as one line of JSON naming its kind and sequence number,
    /// however many lines its text holds.
    ///
    /// # Errors
    /// Returns [`WebError::Frame`] when the message cannot be written as JSON.
    pub(crate) fn json(&self) -> Result<String, WebError> {
        serde_json::to_string(self).map_err(WebError::Frame)
    }
}

#[cfg(test)]
mod tests {
    use muster_domain::{coordination::ScratchpadKey, value::ProjectName};
    use serde_json::Value;

    use super::*;
    use crate::{AgentMark, floor::view::NoteView};

    /// The JSON is one line naming the kind and sequence number, though a
    /// note's body holds a line break.
    #[test]
    fn the_json_is_one_line_naming_its_kind_and_number() {
        let note = NoteView::builder()
            .key(ScratchpadKey::try_new("plan").unwrap())
            .body("two\nlines".to_string())
            .author(AgentMark::Human)
            .label("human".to_string())
            .at(1)
            .build();
        let message = FloorMessage::Notes {
            project: ProjectName::try_new("shop").unwrap(),
            notes: vec![note],
        };

        let json = Sequenced::builder()
            .seq(7)
            .message(message)
            .build()
            .json()
            .unwrap();

        assert!(!json.contains('\n'), "{json}");
        let data: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(data["kind"], "notes");
        assert_eq!(data["seq"], 7);
        assert_eq!(data["project"], "shop");
        assert_eq!(data["notes"][0]["body"], "two\nlines");
        assert_eq!(data["notes"][0]["author"], "human");
    }
}

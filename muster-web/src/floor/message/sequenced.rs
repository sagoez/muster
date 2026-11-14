use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::floor_message::FloorMessage;
use crate::WebError;

/// The field that names a Server-Sent Events frame's event.
const EVENT_FIELD: &str = "event: ";
/// The field that carries a frame's id, which a reconnecting browser sends
/// back.
const ID_FIELD: &str = "id: ";
/// The field that carries a frame's data.
const DATA_FIELD: &str = "data: ";

/// A message with the sequence number the floor gave it, which rises by one
/// per message so the page can drop a stale one.
#[derive(Clone, Debug, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct Sequenced {
    /// Its place in the floor's messages, from 1.
    #[getset(get_copy = "pub")]
    seq: u64,
    /// The message.
    #[getset(get = "pub")]
    #[serde(flatten)]
    message: FloorMessage,
}

impl Sequenced {
    /// The message as one Server-Sent Events frame: its kind as the event, its
    /// sequence number as the id, and one line of JSON holding both as the
    /// data.
    ///
    /// # Errors
    /// Returns [`WebError::Frame`] when the message cannot be written as JSON.
    pub fn frame(&self) -> Result<String, WebError> {
        let data = serde_json::to_string(self).map_err(WebError::Frame)?;
        let kind: &'static str = (&self.message).into();
        Ok(format!(
            "{EVENT_FIELD}{kind}\n{ID_FIELD}{}\n{DATA_FIELD}{data}\n\n",
            self.seq
        ))
    }
}

#[cfg(test)]
mod tests {
    use muster_domain::{coordination::ScratchpadKey, value::ProjectName};
    use serde_json::Value;

    use super::*;
    use crate::{AgentMark, floor::view::NoteView};

    /// A frame is one event whose data is one line of JSON naming its kind
    /// and sequence number, however many lines its text holds.
    #[test]
    fn a_frame_is_one_event_with_one_line_of_data() {
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
        let frame = Sequenced::builder()
            .seq(7)
            .message(message)
            .build()
            .frame()
            .unwrap();

        let lines: Vec<&str> = frame.strip_suffix("\n\n").unwrap().split('\n').collect();
        assert_eq!(lines.len(), 3, "{frame}");
        assert_eq!(lines[0], "event: notes");
        assert_eq!(lines[1], "id: 7");
        let data: Value = serde_json::from_str(lines[2].strip_prefix(DATA_FIELD).unwrap()).unwrap();
        assert_eq!(data["kind"], "notes");
        assert_eq!(data["seq"], 7);
        assert_eq!(data["project"], "shop");
        assert_eq!(data["notes"][0]["body"], "two\nlines");
        assert_eq!(data["notes"][0]["author"], "human");
    }
}

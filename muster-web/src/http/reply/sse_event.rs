use getset::Getters;
use serde_json::Value;
use typed_builder::TypedBuilder;

/// One server-sent event a test read off a stream.
#[derive(Clone, Debug, PartialEq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct SseEvent {
    /// Its `event:` name.
    kind: String,
    /// Its `id:`, when it carries one.
    id: Option<String>,
    /// Its `data:`, read as JSON.
    data: Value,
}

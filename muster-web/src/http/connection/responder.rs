use super::request::Request;
use crate::http::reply::answer::Answer;

/// What answers each request the server reads whole.
pub trait Responder: Send + Sync {
    /// The answer to `request`.
    fn respond(&self, request: &Request) -> Answer;
}
